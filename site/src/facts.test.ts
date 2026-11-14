// @vitest-environment node
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { facts } from "./facts";

it("version matches the workspace Cargo.toml", () => {
  const toml = readFileSync(new URL("../../Cargo.toml", import.meta.url), "utf8");
  const section = toml.split("[workspace.package]")[1] ?? "";
  const match = /^version = "(.+)"/m.exec(section);
  expect(match?.[1]).toBe(facts.version);
});

it("debUrl names the version", () => {
  expect(facts.debUrl.endsWith(`/v${facts.version}/hoprs_${facts.version}-1_amd64.deb`)).toBe(true);
});

it("asset names follow the release patterns", () => {
  const v = facts.version;
  expect(facts.dmgName).toBe(`hoprs_${v}_macos_universal.dmg`);
  expect(facts.msiName).toBe(`hoprs_${v}_windows_x64.msi`);
  expect(facts.zipName).toBe(`hoprs_${v}_windows_x64.zip`);
  for (const [url, name] of [[facts.dmgUrl, facts.dmgName], [facts.msiUrl, facts.msiName], [facts.zipUrl, facts.zipName]]) {
    expect(url.endsWith(`/v${v}/${name}`)).toBe(true);
  }
});
