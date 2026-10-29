import { createServer, type AddressInfo } from 'node:net';
import { defineConfig } from 'playwright/test';
// SITE_TEST_PORT, else a free port picked once by the runner; test workers inherit it.
async function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer().once('error', reject).listen(0, '127.0.0.1', () => {
      const { port } = server.address() as AddressInfo;
      server.close(() => resolve(port));
    });
  });
}
process.env.SITE_TEST_PORT ||= String(await freePort());
const baseURL = `http://127.0.0.1:${process.env.SITE_TEST_PORT}`;
export default defineConfig({
  testDir: './tests',
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  workers: 2,
  retries: 0,
  timeout: 45000,
  expect: { timeout: 10000 },
  reporter: [['list']],
  outputDir: './test-results/artifacts',
  use: { browserName: 'chromium', headless: true, baseURL, trace: 'retain-on-failure' },
  // --ignore-lock keeps Astro's preview in the foreground (it backgrounds itself when run by an agent).
  webServer: { command: `npm run preview -- --host 127.0.0.1 --port ${process.env.SITE_TEST_PORT} --ignore-lock`, url: baseURL, reuseExistingServer: false, timeout: 30000 },
});
