import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// 【重要】API へは開発サーバのプロキシ経由で行く。
// 画面側に API の場所を焼き込まない。第7段階で API Gateway の URL に変わっても、
// ここ1か所で済む。
export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    proxy: { '/api': 'http://127.0.0.1:8090' },
  },
});
