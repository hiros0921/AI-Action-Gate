<script>
  import { onMount, onDestroy } from 'svelte';
  import { approverId, setApproverId, getSettings, getQueue, getHistory, getAudit } from './lib/api.js';
  import Queue from './lib/Queue.svelte';
  import History from './lib/History.svelte';
  import Audit from './lib/Audit.svelte';
  import Simulator from './lib/Simulator.svelte';

  let tab = $state('queue');
  let approver = $state(approverId());
  let editingApprover = $state(!approverId());
  let settings = $state(null);
  let queue = $state([]);
  let history = $state([]);
  let audit = $state([]);
  let lastSync = $state(null);
  let error = $state('');
  let timer;

  // 【重要】間隔はサーバから来ます（/api/settings）。ここに数字を書きません。
  // 304 でも Lambda の呼び出しは数えられるので、間隔はそのまま費用に効きます。
  async function poll() {
    try {
      const next = await getQueue();
      // null は「変わっていない」。再描画もしないので、開いたままでも点滅しません。
      if (next !== null) queue = next;
      lastSync = new Date();
      error = '';
    } catch (e) {
      error = String(e.message ?? e);
    }
  }

  async function loadRest() {
    try {
      [history, audit] = await Promise.all([getHistory(), getAudit()]);
    } catch (e) {
      error = String(e.message ?? e);
    }
  }

  onMount(async () => {
    try {
      settings = await getSettings();
    } catch (e) {
      error = `サーバに繋がりません（${e.message}）`;
      return;
    }
    await poll();
    await loadRest();
    timer = setInterval(async () => {
      await poll();
      if (tab !== 'queue') await loadRest();
    }, settings.poll_interval_ms);
  });

  onDestroy(() => clearInterval(timer));

  function saveApprover() {
    if (!approver.trim()) return;
    setApproverId(approver);
    editingApprover = false;
  }

  async function afterDecision() {
    await poll();
    await loadRest();
  }
</script>

<header>
  <div class="title">
    <h1>AI Action Gate</h1>
    <span class="sub">承認ダッシュボード</span>
  </div>

  <!--
    【重要】誰として操作しているかを常に出す（諏訪の指示・第2段階）。
    「誰が承認したかが見えないと、監査ログの意味が薄くなる」。
  -->
  <div class="who">
    {#if editingApprover}
      <input
        placeholder="承認者ID（例: suwa）"
        bind:value={approver}
        onkeydown={(e) => e.key === 'Enter' && saveApprover()} />
      <button onclick={saveApprover} disabled={!approver.trim()}>この名前で操作する</button>
    {:else}
      <span class="badge"><b>{approver}</b> として操作しています</span>
      <button class="link" onclick={() => (editingApprover = true)}>変更</button>
    {/if}
  </div>
</header>

{#if !approver}
  <p class="warn">
    承認者IDを入力してください。<b>認証は意図的に実装していません</b>が、
    「誰が承認したか」が空の記録は作れないようにしてあります（サーバが 401 を返します）。
  </p>
{/if}

{#if error}
  <p class="error">{error}</p>
{/if}

<nav>
  <button class:active={tab === 'queue'} onclick={() => (tab = 'queue')}>
    承認待ち {queue.length > 0 ? `(${queue.length})` : ''}
  </button>
  <button class:active={tab === 'history'} onclick={() => (tab = 'history')}>履歴</button>
  <button class:active={tab === 'audit'} onclick={() => (tab = 'audit')}>監査ログ</button>
  <button class:active={tab === 'simulate'} onclick={() => (tab = 'simulate')}>閾値シミュレーション</button>

  {#if settings}
    <span class="meta">
      基準 {settings.policy_label}（{settings.policy_version}）
      MEDIUM {settings.thresholds.medium_at} / HIGH {settings.thresholds.high_at}
      {#if !settings.adopted}<b class="warn-inline">未採用の値です</b>{/if}
      ／ {settings.poll_interval_ms / 1000}秒ごとに更新
      {#if lastSync}（最終 {lastSync.toLocaleTimeString('ja-JP')}）{/if}
    </span>
  {/if}
</nav>

<main>
  {#if tab === 'queue'}
    <Queue items={queue} approver={approver} ondecided={afterDecision} />
  {:else if tab === 'history'}
    <History items={history} />
  {:else if tab === 'audit'}
    <Audit entries={audit} />
  {:else if settings}
    <Simulator current={settings.thresholds} />
  {:else}
    <p>設定を読み込んでいます…</p>
  {/if}
</main>

<style>
  :global(body) {
    font-family: -apple-system, 'Hiragino Sans', 'Noto Sans JP', sans-serif;
    margin: 0;
    padding: 16px;
    line-height: 1.7;
    max-width: 1100px;
    margin-inline: auto;
    color-scheme: light dark;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    border-bottom: 1px solid #8884;
    padding-bottom: 10px;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h1 {
    font-size: 1.1rem;
    margin: 0;
  }
  .sub {
    font-size: 0.8rem;
    color: #888;
  }
  .badge {
    border: 1px solid #8884;
    border-radius: 999px;
    padding: 3px 12px;
    font-size: 0.85rem;
  }
  .who {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  nav {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin: 12px 0;
  }
  nav button {
    padding: 6px 14px;
    border: 1px solid #8884;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font: inherit;
  }
  nav button.active {
    border-color: #d6620f;
    font-weight: 700;
  }
  .meta {
    font-size: 0.76rem;
    color: #888;
    margin-left: auto;
  }
  .warn-inline {
    color: #d66;
  }
  .warn,
  .error {
    border: 1px solid #d66;
    border-radius: 8px;
    padding: 10px 12px;
    font-size: 0.9rem;
  }
  .error {
    color: #d66;
  }
  input {
    padding: 5px 8px;
    font: inherit;
    border: 1px solid #8884;
    border-radius: 6px;
    background: transparent;
    color: inherit;
  }
  button.link {
    background: none;
    border: none;
    color: #d6620f;
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
  }
  .who button {
    padding: 5px 12px;
    border: 1px solid #8884;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font: inherit;
  }
</style>
