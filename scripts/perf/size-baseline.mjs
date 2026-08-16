#!/usr/bin/env node
// Download-size baselines for issue #31.
//
// Reads published release assets straight from the GitHub Releases API — for
// srelens and for a comparison project — and buckets them by (os, arch,
// format) so like is compared with like. Nothing is built or estimated here:
// every number is the byte size of an artifact a user would actually download.
//
// Usage:
//   node scripts/perf/size-baseline.mjs                 # markdown table
//   node scripts/perf/size-baseline.mjs --json          # machine-readable
//   node scripts/perf/size-baseline.mjs --check-regression [--max-growth 15]
//
// --tag / --comparison-tag pin each side to a specific release. Pin BOTH to
// reproduce a dated table: without them each side resolves to whatever is
// latest, so the numbers drift as either project ships.
//
// --check-regression compares the two most recent srelens stable releases and
// exits non-zero if any artifact grew by more than --max-growth percent. Set
// GITHUB_TOKEN to avoid the unauthenticated API rate limit.

const SRELENS_REPO = "srelens/srelens";
const COMPARISON_REPO = "freelensapp/freelens";

/** Artifacts that are not themselves downloads: signatures, digests, SBOMs. */
const NOT_AN_ARTIFACT = /\.(sig|sha256|asc|spdx\.json)$|-sbom|^latest\.json$/i;

/**
 * Classify a release asset filename into a comparable bucket. Returns null for
 * anything unrecognized rather than guessing — an unclassified artifact is
 * reported as skipped, never silently folded into another bucket.
 */
export function classifyAsset(name) {
  if (NOT_AN_ARTIFACT.test(name)) return null;
  // "portable" builds are a different distribution shape; keep them out of the
  // installer comparison rather than double-counting a platform.
  if (/portable/i.test(name)) return null;

  const format = name.match(/\.(dmg|deb|rpm|AppImage|msi|exe|pkg)$/i)?.[1];
  if (!format) return null;
  // A .tar.gz app bundle is an update payload, not an installer.
  if (/\.app\.tar\.gz$/i.test(name)) return null;

  const os = /macos|darwin|\.dmg$|\.pkg$/i.test(name)
    ? "macOS"
    : /windows|win32|\.msi$|-setup\.exe$|\.exe$/i.test(name)
      ? "Windows"
      : /linux|\.deb$|\.rpm$|\.AppImage$/i.test(name)
        ? "Linux"
        : null;
  if (!os) return null;

  const arch = /aarch64|arm64/i.test(name)
    ? "arm64"
    : /x86_64|amd64|x64/i.test(name)
      ? "x64"
      : null;
  if (!arch) return null;

  // Canonical casing so the table names the artifact the way its ecosystem
  // spells it (".AppImage", not ".appimage").
  const canonical = { dmg: "dmg", deb: "deb", rpm: "rpm", appimage: "AppImage", msi: "msi", exe: "exe", pkg: "pkg" };
  const ext = canonical[format.toLowerCase()];
  return { os, arch, format: ext, key: `${os} ${arch} .${ext}` };
}

/** MiB with one decimal — the unit release pages use. */
export function mib(bytes) {
  return Math.round((bytes / 1024 / 1024) * 10) / 10;
}

/** Percent growth from `before` to `after`, rounded to one decimal. */
export function growthPct(before, after) {
  if (!before) return null;
  return Math.round(((after - before) / before) * 1000) / 10;
}

async function githubJson(path) {
  const headers = { accept: "application/vnd.github+json", "user-agent": "srelens-perf-baseline" };
  if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const res = await fetch(`https://api.github.com${path}`, { headers });
  if (!res.ok) throw new Error(`GitHub API ${path} failed: ${res.status} ${res.statusText}`);
  return res.json();
}

/** How many 100-item pages of releases to walk before giving up. */
const MAX_RELEASE_PAGES = 5;

/**
 * Every release, newest first — drafts included (they need GITHUB_TOKEN).
 *
 * Paginated because release.yml cuts a dev PRERELEASE daily: a single page
 * covers only a few weeks, so the previous STABLE release drops off it once
 * two stables are far enough apart. Missing that baseline used to make the
 * check report "nothing to compare" and pass, publishing an unmeasured
 * release — so `complete` reports whether the walk actually reached the end,
 * and the caller fails closed when it didn't.
 */
async function allReleases(repo) {
  const releases = [];
  for (let page = 1; page <= MAX_RELEASE_PAGES; page++) {
    const batch = await githubJson(`/repos/${repo}/releases?per_page=100&page=${page}`);
    releases.push(...batch);
    if (batch.length < 100) return { releases, complete: true };
  }
  return { releases, complete: false };
}

/** Stable (non-draft, non-prerelease) releases, newest first. */
async function stableReleases(repo) {
  const { releases } = await allReleases(repo);
  return releases.filter((r) => !r.draft && !r.prerelease);
}

/** Bucket a release's assets into { key: {name, bytes} }, plus what was skipped. */
export function bucketAssets(assets) {
  const sizes = {};
  const skipped = [];
  for (const asset of assets) {
    const bucket = classifyAsset(asset.name);
    if (!bucket) {
      if (!NOT_AN_ARTIFACT.test(asset.name)) skipped.push(asset.name);
      continue;
    }
    // Keep the smallest when a bucket somehow collides, and record the name so
    // the table always says exactly which file a number came from.
    if (!sizes[bucket.key] || asset.size < sizes[bucket.key].bytes) {
      sizes[bucket.key] = { name: asset.name, bytes: asset.size };
    }
  }
  return { sizes, skipped };
}

/**
 * Sizes for a repo's latest stable release, or for an explicit tag. The tag
 * form is what a release run needs: the release being built is still a DRAFT
 * until publish-release flips it, and drafts are excluded from the stable
 * list — so without it the published table would describe the PREVIOUS
 * release rather than the artifacts from this run.
 */
async function releaseSizes(repo, tag) {
  if (tag) {
    const { releases } = await allReleases(repo);
    const release = releases.find((r) => r.tag_name === tag);
    if (!release) {
      throw new Error(`${repo} has no release tagged ${tag} (drafts need GITHUB_TOKEN)`);
    }
    return { tag: release.tag_name, ...bucketAssets(release.assets) };
  }
  const [release] = await stableReleases(repo);
  if (!release) throw new Error(`${repo} has no stable release`);
  return { tag: release.tag_name, ...bucketAssets(release.assets) };
}

function markdownTable(srelens, comparison) {
  const keys = Object.keys(srelens.sizes).sort();
  const lines = [
    `| Installer | srelens \`${srelens.tag}\` | Freelens \`${comparison.tag}\` | Difference |`,
    "| --- | ---: | ---: | ---: |",
  ];
  for (const key of keys) {
    const ours = srelens.sizes[key];
    const theirs = comparison.sizes[key];
    lines.push(
      `| ${key} | ${mib(ours.bytes)} MiB | ${theirs ? `${mib(theirs.bytes)} MiB` : "—"} | ${describeRatio(ours?.bytes, theirs?.bytes)} |`,
    );
  }
  return lines.join("\n");
}

/**
 * How our artifact compares to theirs, in words. Branches on the direction
 * rather than always saying "smaller": an unpinned run resolves whatever is
 * latest, so if a future comparison release ships a smaller artifact this must
 * report srelens as LARGER instead of printing "0.8× smaller" and implying a
 * win that isn't there.
 */
export function describeRatio(ourBytes, theirBytes) {
  if (!ourBytes || !theirBytes) return "—";
  if (ourBytes === theirBytes) return "same size";
  const [bigger, smaller] = ourBytes > theirBytes ? [ourBytes, theirBytes] : [theirBytes, ourBytes];
  const factor = Math.round((bigger / smaller) * 10) / 10;
  const direction = ourBytes > theirBytes ? "larger" : "smaller";
  // Below 1.5x a multiple reads oddly ("1.1x smaller" for a 5% gap); percent
  // is the honest way to describe near-parity.
  if (factor < 1.5) {
    const pct = Math.round(Math.abs((ourBytes - theirBytes) / theirBytes) * 1000) / 10;
    return `${pct}% ${direction}`;
  }
  return `${factor}× ${direction}`;
}

async function checkRegression(maxGrowth, tag) {
  // On main the release under test is still a DRAFT (release.yml flips it only
  // after the whole pipeline is green), and drafts are excluded from the
  // stable list. Without --tag this would silently compare the two previous
  // published releases and never see the artifacts just built.
  const { releases, complete } = await allReleases(SRELENS_REPO);
  const current = tag
    ? releases.find((r) => r.tag_name === tag)
    : releases.find((r) => !r.draft && !r.prerelease);
  if (!current) {
    console.error(`No release found for tag ${tag}. Is GITHUB_TOKEN set (drafts need auth)?`);
    return 1;
  }
  const previous = releases.find(
    (r) => !r.draft && !r.prerelease && r.tag_name !== current.tag_name,
  );
  if (!previous) {
    // Only safe to pass when the walk actually saw every release: then there
    // genuinely is no earlier stable one (the first release ever). If the
    // page cap cut the walk short, the baseline may simply be out of reach —
    // that is an unmeasured release, not a clean one.
    if (!complete) {
      console.error(
        `FAIL: no previous stable release within ${MAX_RELEASE_PAGES} pages of releases — ` +
          "the baseline could not be read, so this release was never actually measured.",
      );
      return 1;
    }
    console.log("No previous stable release to compare against.");
    return 0;
  }
  const now = bucketAssets(current.assets).sizes;
  const before = bucketAssets(previous.assets).sizes;
  const verdict = compareBuckets(before, now, maxGrowth);

  console.log(`Size change ${previous.tag_name} → ${current.tag_name}:`);
  console.log(verdict.rows.join("\n") || "  (no comparable artifacts)");
  if (verdict.appeared.length) {
    console.log(`\nNew installers (no previous size to compare): ${verdict.appeared.join(", ")}`);
  }

  if (!verdict.ok) {
    console.error(`\nFAIL: ${verdict.reason}`);
    return 1;
  }
  console.log(`\nOK: largest growth ${verdict.worst}% is within the ${maxGrowth}% budget.`);
  return 0;
}

/**
 * Compare two releases' buckets. Fails CLOSED: a bucket the previous release
 * had and this one doesn't is a failure, not a skip — a renamed installer (a
 * bundler update changing its filename) or an upload that silently dropped one
 * would otherwise leave that artifact unguarded, and if EVERY bucket stopped
 * classifying, an empty comparison would report success and let the release
 * through unmeasured.
 */
export function compareBuckets(before, now, maxGrowth) {
  const keys = [...new Set([...Object.keys(before), ...Object.keys(now)])].sort();
  const rows = [];
  const missing = [];
  const appeared = [];
  let worst = 0;
  let compared = 0;

  for (const key of keys) {
    if (!now[key]) {
      missing.push(key);
      rows.push(`  ${key.padEnd(22)} ${mib(before[key].bytes)} MiB → MISSING`);
      continue;
    }
    if (!before[key]) {
      appeared.push(key);
      continue;
    }
    const pct = growthPct(before[key].bytes, now[key].bytes);
    worst = Math.max(worst, pct);
    compared += 1;
    rows.push(
      `  ${key.padEnd(22)} ${mib(before[key].bytes)} → ${mib(now[key].bytes)} MiB (${pct >= 0 ? "+" : ""}${pct}%)`,
    );
  }

  if (missing.length) {
    return { rows, missing, appeared, worst, ok: false,
      reason: `installer(s) present in the previous release are missing or no longer recognized: ${missing.join(", ")}` };
  }
  if (compared === 0) {
    return { rows, missing, appeared, worst, ok: false,
      reason: "no installers could be compared — the classifier or the upload is broken, so nothing was actually checked" };
  }
  if (worst > maxGrowth) {
    return { rows, missing, appeared, worst, ok: false,
      reason: `an installer grew ${worst}%, over the ${maxGrowth}% budget.` };
  }
  return { rows, missing, appeared, worst, ok: true, reason: "" };
}

async function main() {
  const args = process.argv.slice(2);
  const maxGrowth = Number(args[args.indexOf("--max-growth") + 1]) || 15;
  const arg = (name) => {
    const i = args.indexOf(name);
    return i === -1 ? null : args[i + 1];
  };
  const tag = arg("--tag");
  const comparisonTag = arg("--comparison-tag");

  if (args.includes("--check-regression")) {
    process.exit(await checkRegression(maxGrowth, tag));
  }

  // Each side resolves to its own latest stable unless pinned. A release run
  // pins only --tag (its own draft) and wants the comparison project's
  // current release; a documented table pins both so it stays reproducible.
  const [srelens, comparison] = await Promise.all([
    releaseSizes(SRELENS_REPO, tag),
    releaseSizes(COMPARISON_REPO, comparisonTag),
  ]);

  if (args.includes("--json")) {
    console.log(JSON.stringify({ srelens, comparison, measuredBy: "release asset bytes" }, null, 2));
    return;
  }

  console.log(markdownTable(srelens, comparison));
  for (const [label, data] of [["srelens", srelens], ["comparison", comparison]]) {
    if (data.skipped.length) {
      console.log(`\n<!-- ${label} assets not classified: ${data.skipped.join(", ")} -->`);
    }
  }
}

// Only run when invoked directly, so the helpers above stay unit-testable.
if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
