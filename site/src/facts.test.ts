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
