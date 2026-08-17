<script>
  let { entries = [] } = $props();
</script>

<p class="lead">
  監査ログは<b>追記のみ</b>です。画面からも API からも、直す手段・消す手段を用意していません。
  第7段階では IAM で <code>UpdateItem</code> / <code>DeleteItem</code> を拒否します。
</p>

{#if entries.length === 0}
  <p class="empty">まだ記録がありません。</p>
{:else}
  <table>
    <thead>
      <tr>
        <th>日時</th><th>要求</th><th>誰が</th><th>判断</th>
        <th>自動判定</th><th>そのときの閾値</th><th>検出</th>
      </tr>
    </thead>
    <tbody>
      {#each entries.slice().reverse() as e, i (e.request_id + e.at + i)}
        <tr>
          <td class="note">{e.at.slice(0, 19).replace('T', ' ')}</td>
          <td class="mono">{e.request_id}</td>
          <td>
            {#if e.reviewer.kind === 'system'}
              <span class="system">自動承認</span>
            {:else}
              {e.reviewer.approver_id}
            {/if}
          </td>
          <td>{e.verdict}</td>
          <td>{e.decision} / {e.score}点</td>
          <!--
            【重要】「そのときの閾値」（仕様書7章）。
            あとから閾値を変えても、過去の判断を当時の基準で読み直せる。
          -->
          <td class="mono">{e.thresholds.medium_at}/{e.thresholds.high_at}<br />
            <span class="note">{e.policy_version}</span></td>
          <td class="note">
            {#if e.detected.length === 0}なし{:else}
              {e.detected.map((d) => `${d.kind}×${d.count}`).join('・')}
            {/if}
            {#if !e.conclusive}<b>（走査できず）</b>{/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  table { border-collapse: collapse; width: 100%; font-size: 0.86rem; }
  th, td { border-bottom: 1px solid #8884; padding: 5px 7px; text-align: left; vertical-align: top; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.78rem; }
  .note { font-size: 0.78rem; color: #888; }
  .system { color: #888; }
  .lead { font-size: 0.86rem; }
  .empty { color: #888; }
  code { font-size: 0.8rem; }
</style>
