import { defineConfig } from 'astro/config';
export default defineConfig({
  site: 'https://matrust.dev',
  base: '/',
  output: 'static',
  trailingSlash: 'always',
  devToolbar: { enabled: false },
});
