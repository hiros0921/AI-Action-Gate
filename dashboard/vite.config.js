import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// 【重要】API へは開発サーバのプロキシ経由で行く。
// 画面側に API の場所を焼き込まない。AWS 上の Lambda に向け先が変わっても、
// ここ1か所で済む。実際、第7段階で Function URL に向けたときの変更はこの3行だけだった。
//
//   GATE_API=https://xxxx.lambda-url.ap-northeast-1.on.aws npm run dev
//
// changeOrigin は Lambda Function URL 側で必要。Host ヘッダが localhost のままだと
// 署名・ルーティングの前提が崩れる。
const target = process.env.GATE_API ?? 'http://127.0.0.1:8090';

export default defineConfig({
  plugins: [svelte()],
  server: {
    port: 5173,
    proxy: { '/api': { target, changeOrigin: true } },
  },
});
