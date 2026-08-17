<script>
  import { simulate } from './api.js';

  // 【重要】いまの閾値は必ず渡されます（App が settings を読んでから描画する）。
  // ここに既定値を書かないこと。書くと採用値が画面側に複製され、
  // サーバの設定を変えても画面が古い値を持ち続けます。
  let { current } = $props();

  let mediumAt = $state(current.medium_at);
  let highAt = $state(current.high_at);
  let result = $state(null);
  let error = $state('');
  let busy = $state(false);

  // 動かすたびに測り直す。本文は読み直さないので、何度動かしても安い。
  $effect(() => {
    const m = mediumAt;
    const h = highAt;
    let cancelled = false;
    busy = true;
    simulate(m, h)
      .then((r) => {
        if (!cancelled) {
          result = r;
          error = '';
        }
      })
      .catch((e) => {
        if (!cancelled) {
          error = String(e.message ?? e);
          result = null;
        }
      })
      .finally(() => {
        if (!cancelled) busy = false;
      });
    return () => {
      cancelled = true;
    };
  });
</script>

<p class="lead">
  <b>閾値を変えたら、過去の要求が何件どちらに動くか。</b>
  ここがこのシステムの中心です。全部を人の承認にすれば安全ですが、誰も使いません。
  全部を自動にすれば速いですが、事故が起きます。その境目を数字で調整するための画面です。
</p>
<p class="note">
  再判定に<b>本文は読み直していません</b>。保存してある内訳（要素・素点・重み）だけで計算しています。
  だから平文を消したあとでもシミュレーションできます。
</p>

<div class="controls">
  <label>
    MEDIUM の境目 <b>{mediumAt}</b>
    <input type="range" min="0" max="100" bind:value={mediumAt} />
  </label>
  <label>
    HIGH の境目 <b>{highAt}</b>
    <input type="range" min="0" max="100" bind:value={highAt} />
  </label>
  <button class="link" onclick={() => { mediumAt = current.medium_at; highAt = current.high_at; }}>
    いまの設定に戻す（{current.medium_at} / {current.high_at}）
  </button>
</div>

{#if error}
  <p class="error">{error}</p>
{:else if result}
  <table>
    <thead>
      <tr><th></th><th class="n">LOW（自動）</th><th class="n">MEDIUM</th><th class="n">HIGH</th><th class="n">人手に回る割合</th></tr>
    </thead>
    <tbody>
      <tr>
        <th>いま</th>
        <td class="n">{result.current.low}</td>
        <td class="n">{result.current.medium}</td>
        <td class="n">{result.current.high}</td>
        <td class="n">{result.current.human_rate}%</td>
      </tr>
      <tr class="proposed">
        <th>変更後</th>
        <td class="n">{result.proposed.low}</td>
        <td class="n">{result.proposed.medium}</td>
        <td class="n">{result.proposed.high}</td>
        <td class="n">{result.proposed.human_rate}%</td>
      </tr>
    </tbody>
  </table>

  <p class="moved">
    自動承認へ移るのは <b>{result.moving_to_auto.count}件</b>（全{result.total}件中）。
    {#if result.moving_to_auto.with_pii > 0}
      <!--
        【重要】件数だけ見て緩めると、緩めてはいけないものが混ざる。
        だから「そのうちPIIを含む件数」を必ず並べて出す。
      -->
      <span class="danger">うち <b>{result.moving_to_auto.with_pii}件</b> は PII を含みます。</span>
    {:else if result.moving_to_auto.count > 0}
      PII を含むものはありません。
    {/if}
  </p>

  {#if busy}<p class="note">計算中…</p>{/if}
{:else}
  <p class="note">計算中…</p>
{/if}

<style>
  .lead { font-size: 0.92rem; }
  .note { font-size: 0.8rem; color: #888; }
  .controls { display: flex; gap: 24px; align-items: center; flex-wrap: wrap; margin: 14px 0; }
  label { font-size: 0.88rem; display: flex; flex-direction: column; gap: 2px; }
  input[type='range'] { width: 260px; }
  table { border-collapse: collapse; margin-top: 10px; font-size: 0.9rem; }
  th, td { border-bottom: 1px solid #8884; padding: 6px 14px; text-align: left; }
  .n { text-align: right; }
  tr.proposed { font-weight: 700; }
  .moved { margin-top: 10px; }
  .danger { color: #d66; }
  .error { color: #d66; border: 1px solid #d66; border-radius: 8px; padding: 8px 12px; }
  button.link {
    background: none; border: none; color: #d6620f;
    cursor: pointer; font: inherit; text-decoration: underline;
  }
</style>
