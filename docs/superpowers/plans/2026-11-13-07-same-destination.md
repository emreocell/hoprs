# Plano 7: Um destino, um escritor — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** dois itens da fila nunca gravam o mesmo arquivo de destino ao mesmo tempo. O segundo espera o primeiro terminar e então passa pela checagem de conflito normal.

**Architecture:** o ator do motor (`crates/hoprs-transfer/src/engine/actor.rs`) passa a manter, para cada destino, qual item em execução o está gravando. No despacho, um item `Queued` cujo destino está ocupado é pulado: continua na fila, e o próximo item elegível do mesmo site sai no lugar dele. Quando o dono do destino libera o slot, o despacho já existente roda de novo, e o item adiado sai. Como o primeiro já criou o arquivo, a checagem de conflito do segundo (`stat` no preparo) agora o encontra e aplica a política (`Ask`, `Skip`...).

**Tech Stack:** Rust; só o `hoprs-transfer`.

**Spec:** [`docs/superpowers/specs/2026-11-11-hoprs-design.md`](../specs/2026-11-11-hoprs-design.md) §5 (conflitos: política por fila, "ask" inline). Pendência: [`2026-11-12-01-followups.md`](2026-11-12-01-followups.md), Plan 6 → Correctness, "First to do".

## Global Constraints

- **Nenhuma API pública muda:** nem `TransferEngine`, nem `Event`, nem o esquema SQLite.
- **Sem sobrescrita calada:** um arquivo existente nunca é sobrescrito sem passar pela política de conflito, a não ser que a política seja `Overwrite` (spec §5).
- **Ordem de despacho preservada:** arquivos antes de pastas, menor id primeiro, por site. A única mudança é pular itens cujo destino está ocupado.
- **Custo:** o despacho continua O(itens despachados + itens pulados) por chamada, sem varrer a fila inteira para cada slot livre.
- `cargo fmt --all --check` e `cargo clippy --workspace --all-targets --all-features -- -D warnings` limpos.

## Review Focus

1. **Duas árvores enviadas para a mesma pasta**, cada uma com um arquivo `x`: o segundo `x` cai na política (`Ask` vira `AwaitingDecision`, `Skip` vira `Skipped`) e o conteúdo do primeiro fica intacto. → Task 1.
2. **Dois downloads, de sites diferentes, para o mesmo arquivo local:** também são serializados, porque o destino local é um só. → Task 1.
3. **Dois uploads para o mesmo caminho em sites diferentes:** **não** são serializados, porque são servidores diferentes. → Task 1.
4. **Item adiado enquanto o dono pausa, falha ou é cancelado:** o destino é liberado e o adiado sai, sem ficar preso. → Task 1.
5. **Fila longa com muitos adiados:** o despacho não trava nem entra em laço, e os outros itens do site continuam saindo. → Task 1.

---

### Task 1: Serializar itens por destino

**Files:**
- Modify: `crates/hoprs-transfer/src/engine/actor.rs` (`SiteQueue`, `dispatch`, `free_slot`, campos do `Actor`)
- Test: `crates/hoprs-transfer/tests/engine_conflicts.rs` (ou o arquivo de testes de conflito existente), mais unitários em `actor.rs`

**Interfaces:**
- **`enum DestKey { Remote(SiteId, RemotePath), Local(RemotePath) }`:** um upload grava `Remote(spec.site, spec.remote)`, e um download grava `Local(spec.local)`. Vem de `fn dest_key(spec: &TransferSpec) -> DestKey`, ao lado de `destination`.
- **`Actor.writing: HashMap<DestKey, TransferId>`:** o item que segura o slot e grava aquele destino.
  - É inserido no `dispatch` quando o item vira `Active`.
  - É removido em `free_slot`, que é chamado em todo fim de execução (`Done`, `Failed`, `Paused`, `Cancelled`, `AwaitingDecision`, requeue de rede).
  - Um item `AwaitingDecision` **não** segura o destino, porque ainda não escreveu nada. Se o usuário escolher `Overwrite` depois que outro item gravou, a escolha foi dele.
  - Uma pasta (`is_dir`) também registra o seu destino, que é exato e não por prefixo. Isso evita que um arquivo e uma pasta com o mesmo caminho sejam preparados juntos.
- **`SiteQueue::pop_eligible(&mut self, busy: impl Fn(TransferId) -> bool) -> Option<TransferId>`:** devolve o primeiro id, na ordem atual (arquivos, depois pastas, menor id), para o qual `busy` é falso, e o tira da fila. Os ids pulados continuam na fila.
  - O `dispatch` usa `pop_eligible` com `busy = |id| writing.contains_key(&dest_key(items[id]))`.
  - Os itens pulados num despacho são percorridos de novo no próximo. Isso é aceitável, porque o número de destinos ocupados é limitado pelo número de workers.
- **Despacho de outros sites:** `free_slot` não muda de assinatura. O `release` já chama `dispatch` depois de liberar. Se o dono e o adiado forem de sites diferentes (downloads para o mesmo arquivo local), o `dispatch` já percorre todos os sites com slot livre, então o adiado sai.

- [ ] **Step 1: Testes (falhando).**
  - `two_trees_into_one_new_folder_never_mix_bytes` **(Review Focus 1)**. Com `MemFs` e `FaultFs` usando latência de stream para forçar sobreposição, duas raízes enviam `x` para `/srv/tree/x`. Verificar:
    - com `Ask`, uma delas vira `AwaitingDecision`;
    - com `Skip`, ela vira `Skipped`;
    - o arquivo remoto é exatamente o primeiro conteúdo, e nenhum item falha com "Size mismatch".
  - `downloads_from_two_sites_to_one_local_file_are_serialized` **(Review Focus 2)**.
  - `uploads_to_the_same_path_on_two_sites_run_in_parallel` **(Review Focus 3)**: os dois ficam `Active` ao mesmo tempo.
  - `deferred_item_starts_when_the_owner_pauses_fails_or_is_cancelled` **(Review Focus 4)**: três casos.
  - `many_deferred_items_do_not_block_other_items` **(Review Focus 5)**: 100 itens com o mesmo destino e 5 itens distintos no mesmo site, com 4 workers. Os distintos terminam e os 100 saem um por vez, sem laço. Asserção de tempo generosa.
  - Unitário `pop_eligible_keeps_order_and_skipped_ids`.
- [ ] **Step 2: Rodar e ver falhar.** Run: `cargo test -p hoprs-transfer`
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Rodar e ver passar.** Run: `cargo test --workspace --all-features` (3×, por causa dos testes de tempo) e a suíte `--ignored` com Docker uma vez.
- [ ] **Step 5: Atualizar a pendência.** Em `docs/superpowers/plans/2026-11-12-01-followups.md`, marcar o item "First to do" como **Resolved in Plan 7**, com uma linha explicando como.
- [ ] **Step 6: Commit.** `git commit -m "fix(transfer): one writer per destination"`

## Critério de conclusão

- Testes da Review Focus passando.
- Workspace, suíte `--ignored` e CI verdes.
- Pendência marcada como resolvida.
- Próximo passo: plano do release 1.0.0 (`.deb` via cargo-deb, ícone, `.desktop`, workflow de release no GitHub disparado por tag `v1.0.0`).
