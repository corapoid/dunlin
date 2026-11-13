import { defineConfig } from 'vite';
import { readFileSync } from 'node:fs';

// Fixed loopback backend, no externally supplied target or permissive CORS.
// Keep Host AND Origin unchanged: the backend checks their exact relationship.
export default defineConfig({
  plugins: [{
    name: 'dunlin-reui-notice',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: 'REUI-LICENSE.txt', source: readFileSync(new URL('./REUI-LICENSE.txt', import.meta.url), 'utf8') });
    },
  }],
  resolve: { alias: { '@': new URL('./src', import.meta.url).pathname } },
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/v1/web': { target: 'http://127.0.0.1:8787', ws: true, changeOrigin: false },
    },
  },
  build: { sourcemap: false },
});
