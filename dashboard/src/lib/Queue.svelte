<script>
  import { decide } from './api.js';

  let { items = [], approver = '', ondecided } = $props();

  let open = $state(null);
  let busy = $state(null);
  let error = $state('');
  /** 承認前に伏せ字を見たか。マスキング承認のときに効く。 */
  let showMasked = $state({});

  const riskLabel = { LOW: '自動承認', MEDIUM: '承認待ち', HIGH: '承認待ち（要マスキング検討）' };

  async function act(id, verdict) {
    busy = id;
    error = '';
    try {
      await decide(id, verdict);
      open = null;
      await ondecided?.();
    } catch (e) {
      error = String(e.message ?? e);
    } finally {
      busy = null;
    }
  }
</script>

{#if error}<p class="error">{error}</p>{/if}

{#if items.length === 0}
  <p class="empty">承認待ちはありません。</p>
{:else}
  {#each items as item (item.request_id)}
    <article class:high={item.risk === 'HIGH'}>
      <header>
        <span class="risk {item.risk}">{item.risk}</span>
        <b>{item.score}点</b>
        {#if item.clamped}
          <!-- 【重要】100で頭打ちになった事実を隠さない。監査上、危険要因の量が違う。 -->
          <span class="note">（切る前 {item.raw_total}点）</span>
        {/if}
        <span class="who">{item.agent_id}</span>
        <span class="note">{riskLabel[item.risk]}</span>
        <button class="link" onclick={() => (open = open === item.request_id ? null : item.request_id)}>
          {open === item.request_id ? '閉じる' : '内訳を見る'}
        </button>
      </header>

      {#if !item.conclusive}
        <p class="uncertain">
          <b>本文を走査できていません。</b>
          PIIが無いという意味ではありません。点が低くても人手に回してあります。
        </p>
      {:else if item.raised_by_uncertainty}
        <p class="uncertain">
          確証のない氏名を含みます。<b>「氏名かもしれない」は「氏名がない」とは違う</b>ので、
          点が低くても人手に回してあります。
        </p>
      {/if}

      <p class="payload">{showMasked[item.request_id] ? item.masked_preview : item.payload}</p>
      <label class="toggle">
        <input type="checkbox" bind:checked={showMasked[item.request_id]} />
        マスキング後を表示
      </label>

      {#if item.detected.length > 0}
        <p class="detected">
          検出:
          {#each item.detected as d}
            <span class="pill" class:weak={d.lowest_confidence !== 'high'}>
              {d.kind} ×{d.count}
              {#if d.lowest_confidence !== 'high'}（確信度 {d.lowest_confidence}）{/if}
            </span>
          {/each}
        </p>
      {/if}

      {#if open === item.request_id}
        <table>
          <thead>
            <tr><th>要素</th><th class="n">素点</th><th class="n">重み</th><th class="n">寄与</th><th>なぜその点か</th></tr>
          </thead>
          <tbody>
            {#each item.components as c}
              <tr>
                <td>{c.matched}</td>
                <td class="n">{c.raw}</td>
                <td class="n">×{c.weight}</td>
                <td class="n">{c.points.toFixed(1)}</td>
                <td class="why">{c.why}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="note">
          基準 {item.policy_version}（MEDIUM {item.thresholds.medium_at} / HIGH {item.thresholds.high_at}）
          ／ 受付 {item.created_at.slice(0, 19).replace('T', ' ')}
        </p>
      {/if}

      <footer>
        <button class="danger" disabled={!approver || busy} onclick={() => act(item.request_id, 'reject')}>
          拒否
        </button>
        <button disabled={!approver || busy} onclick={() => act(item.request_id, 'approve_masked')}>
          マスキングして承認
        </button>
        <button class="ok" disabled={!approver || busy} onclick={() => act(item.request_id, 'approve')}>
          承認
        </button>
        {#if busy === item.request_id}<span class="note">送信中…</span>{/if}
      </footer>
    </article>
  {/each}
{/if}

<style>
  article {
    border: 1px solid #8884;
    border-radius: 8px;
    padding: 12px 14px;
    margin-bottom: 12px;
  }
  article.high {
    border-color: #d66;
  }
  header {
    display: flex;
    gap: 10px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .risk {
    font-weight: 700;
    font-size: 0.78rem;
    border-radius: 4px;
    padding: 1px 8px;
    border: 1px solid currentColor;
  }
  .risk.HIGH {
    color: #d66;
  }
  .risk.MEDIUM {
    color: #d6620f;
  }
  .who {
    font-size: 0.8rem;
    color: #888;
  }
  .note {
    font-size: 0.78rem;
    color: #888;
  }
  .payload {
    white-space: pre-wrap;
    border-left: 3px solid #8884;
    padding-left: 10px;
    margin: 10px 0 4px;
  }
  .toggle {
    font-size: 0.8rem;
    color: #888;
  }
  .uncertain {
    font-size: 0.85rem;
    border: 1px dashed #d6620f;
    border-radius: 6px;
    padding: 6px 10px;
  }
  .detected {
    font-size: 0.82rem;
  }
  .pill {
    border: 1px solid #8884;
    border-radius: 999px;
    padding: 1px 9px;
    margin-right: 5px;
  }
  .pill.weak {
    border-style: dashed;
    color: #d6620f;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.85rem;
    margin: 8px 0;
  }
  th,
  td {
    border-bottom: 1px solid #8884;
    padding: 4px 6px;
    text-align: left;
    vertical-align: top;
  }
  .n {
    text-align: right;
    white-space: nowrap;
  }
  .why {
    color: #888;
    font-size: 0.8rem;
  }
  footer {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
  }
  button {
    padding: 6px 14px;
    border: 1px solid #8884;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font: inherit;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  button.danger {
    border-color: #d66;
    color: #d66;
  }
  button.ok {
    border-color: #2a2;
  }
  button.link {
    border: none;
    color: #d6620f;
    text-decoration: underline;
    padding: 0;
    margin-left: auto;
  }
  .empty {
    color: #888;
  }
  .error {
    color: #d66;
    border: 1px solid #d66;
    border-radius: 8px;
    padding: 8px 12px;
  }
</style>
