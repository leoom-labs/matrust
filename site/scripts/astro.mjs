// Runs the Astro CLI with telemetry off: the site's tooling sends nothing and never
// writes to the user's config folder (read-only in sandboxed gates).
process.env.ASTRO_TELEMETRY_DISABLED = '1';
await import('../node_modules/astro/bin/astro.mjs');
