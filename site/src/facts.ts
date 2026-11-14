// Filled by hand per release. facts.test.ts checks the version against the workspace Cargo.toml.
const repoUrl = "https://github.com/engviniciusbs/hoprs";
const version = "1.0.0";
const debName = `hoprs_${version}-1_amd64.deb`;
const releaseBase = `${repoUrl}/releases/download/v${version}`;

export const facts = {
  version,
  debName,
  debUrl: `${releaseBase}/${debName}`,
  sumsUrl: `${releaseBase}/SHA256SUMS`,
  repoUrl,
  releasesUrl: `${repoUrl}/releases`,
  changelogUrl: `${repoUrl}/blob/main/CHANGELOG.md`,
  securityUrl: `${repoUrl}#security`,
  perfDocUrl: `${repoUrl}/blob/main/docs/perf/README.md`,
  licenseUrl: `${repoUrl}/blob/main/LICENSE-MIT`,
  issuesUrl: `${repoUrl}/issues`,
  perf: { startupMs: 84, idleMb: 33, rowsP95Ms: 0.58, rows: 100000, sftpVsScp: 0.99, measuredOn: "2026-11-13" },
} as const;
