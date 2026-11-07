import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// `pnpm dev` proxies the API to a daemon on the default port.
const daemon = process.env.XHERD_URL ?? 'http://127.0.0.1:4740';

export default defineConfig({
  plugins: [svelte()],
  build: { outDir: 'dist', emptyOutDir: true, chunkSizeWarningLimit: 1500 },
  server: {
    proxy: {
      '/api': { target: daemon, ws: true, changeOrigin: false },
    },
  },
  test: { include: ['src/**/*.test.ts'] },
});
