<script>
  let { items = [] } = $props();

  const verdictLabel = {
    approved: '承認',
    approved_with_masking: 'マスキングして承認',
    rejected: '拒否',
  };
</script>

{#if items.length === 0}
  <p class="empty">まだ判断された要求はありません。</p>
{:else}
  <table>
    <thead>
      <tr>
        <th>判断</th><th>要求</th><th class="n">点</th><th>自動判定</th>
        <th>承認者</th><th>検出</th><th>日時</th>
      </tr>
    </thead>
    <tbody>
      {#each items as it (it.request_id)}
        <tr>
          <td class="verdict {it.verdict}">{verdictLabel[it.verdict] ?? it.verdict}</td>
          <td class="mono">{it.request_id}<br /><span class="note">{it.agent_id}</span></td>
          <td class="n">{it.score}</td>
          <td>
            {it.risk}
            <!--
              【重要】人が機械の判定を覆したことを見えるようにする。
              HIGH をそのまま承認した記録は、あとから必ず問われる。
            -->
            {#if it.overrode_machine}<span class="override">← 人が覆した</span>{/if}
          </td>
          <td>{it.reviewer}</td>
          <td class="note">
            {#if it.detected.length === 0}なし{:else}
              {it.detected.map((d) => `${d.kind}×${d.count}`).join('・')}
            {/if}
          </td>
          <td class="note">{it.decided_at.slice(0, 19).replace('T', ' ')}</td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="note">
    平文は載せていません。載っているのは検出の<b>種別と件数だけ</b>です（仕様書7章）。
  </p>
{/if}

<style>
  table { border-collapse: collapse; width: 100%; font-size: 0.86rem; }
  th, td { border-bottom: 1px solid #8884; padding: 5px 7px; text-align: left; vertical-align: top; }
  .n { text-align: right; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.8rem; }
  .note { font-size: 0.78rem; color: #888; }
  .verdict { font-weight: 700; white-space: nowrap; }
  .verdict.rejected { color: #d66; }
  .verdict.approved_with_masking { color: #d6620f; }
  .override { color: #d66; font-size: 0.75rem; }
  .empty { color: #888; }
</style>
