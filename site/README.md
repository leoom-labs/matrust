# MatRust website

Static Astro site for https://matrust.dev. Requires Node >=22.23.4 (Node 26 recommended).
No runtime UI framework, external fonts, analytics or cookies.

```sh
npm ci
npm run dev
```

The production acceptance sequence is `npm run check && npm run build && npm test`.
Playwright is pinned to 1.63.0 (Chromium revision 1243): `npx playwright install chromium`
fetches that build if the default cache lacks it. The browser suite renders the six
static pages and six sample function pages (linspace, norm, containers.Map, annotation,
isequalwithequalnans, plot) at both viewport sizes and themes, with screenshots in
`test-results/screenshots/`; a cheap check reads every built function page (heading
present, no double-escaped text). Two workers bound concurrency. Tests use the built
site through `astro preview` on `SITE_TEST_PORT` or a free port; no real screen is
used. `npm run proof` runs focused source checks without building the site.

npm scripts run Astro through `scripts/astro.mjs`, which turns Astro's telemetry off.
Astro 7 moves `astro preview` to the background when it detects an AI agent; pass
`--ignore-lock` to keep it in the foreground (the test server does).

## Engine integration

`npm run build` calls `scripts/copy-engine.mjs`. If `../web/pkg/` exists, its files
are copied into `public/engine/`; otherwise any engine files supplied there are kept.
The bundle must contain `worker.js`, `matrust_web.js`, and `matrust_web_bg.wasm`.
The console speaks the task's `{type,id,code?}` / `{id,result}` worker contract.
`run` and `workspace` results are JSON strings. No files or browser filesystem are used.

The first Run fetches the worker URL. A 404 (or an HTML fallback) selects the bundled
mock worker and displays **Preview engine: results are canned**. Other download failures
remain errors, so an unavailable real engine is not misrepresented as a working preview.
The mock never evaluates code; its output, workspace and figure are canned. Wasm loads
with a progress reader before the worker starts. Size is derived from generated `engine/size.json`, response headers,
then actual transferred bytes; it is not an estimated claim about an unavailable bundle.
The worker's subsequent Wasm request can use the browser HTTP cache. Caching and size
headers depend on the host; browsers can evict cached files. Runs time out after 10 s
once the worker receives the request. Stop terminates and recreates an existing worker,
clearing the session. Reset remains lazy before the first run. After the worker loads
(it answers a `version` request first), a worker error is reported as "The engine
stopped. The workspace was cleared." and the next Run starts a fresh worker; a worker
that never loaded is reported as a load failure and downloaded again on the next Run.

Share links use `#code=v1.<base64url UTF-8>` or `#code=v1z.<base64url deflate>`.
CompressionStream is used when present and smaller. Decode has a 64 KB expansion
limit. Code is never uploaded. SVG results are rendered as blob-backed images, not
HTML, and downloaded as the original SVG. Code that produces figures should only be
shared with people who expect the generated SVG files.

## Reference generation

`src/lib/functions.ts` reads `../assets/help/functions.md` at build time. It splits at
literal level-two headings, takes the first nonblank line as the summary, extracts the
first `Example:` line, and backslash-escapes every ASCII punctuation mark of the plain
help text so marked renders each line verbatim (marked does the HTML escaping).
`Syntax:` and `Example:` lines and indented listings become code blocks. Source evidence and compatibility
limitations are retained. There are 767 sections and 764 distinct names. `interp1`,
`norm`, and `isequal` occur twice: each gets one URL with both entries in source order,
an explicit duplication notice, the final summary for the search index and the first
example for its Try link. `containers.Map` keeps its dotted name in its URL.
No upstream help files are changed. The JSON search index is generated with the pages.

## Release data and assets

`npm run refresh-release [tag]` refreshes the pinned fallback (default: the version in
`src/data/release.json`) from the public release
API and each archive's `.sha256` file. It validates the checksum filename and format.
Browser refresh tries `/releases/latest`, then `/releases?per_page=1`, and caches valid
complete release data for ten minutes. Live assets need a GitHub SHA-256 `digest`, or
an exact match to the bundled checksum. Incomplete releases retain the bundled
cards; no checksum is guessed or paired with a different archive. Runtime requests
are restricted to GitHub's release API and self-hosted files.

Fonts and screenshots are the supplied licensed files. `scripts/render-social.mjs`
uses bundled Inter and headless Chromium to reproduce the committed 1200×630 social
card (`node scripts/render-social.mjs`). No external images or SolidCraft assets are used. `node scripts/render-logo.mjs` extracts
the M outline directly from the bundled Inter SemiBold font for the logo and favicon.
The favicon and logo use MatRust's accent square and white M.

Hosting, DNS and copying the real engine bundle are handled by the orchestrator.
