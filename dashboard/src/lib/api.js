/**
 * API との通信。
 *
 * 【重要】ポーリング間隔をここに書きません（諏訪の指示・第5段階）。
 * サーバの /api/settings から取ります。304 でも Lambda の呼び出しは数えられるので、
 * 間隔はそのまま費用に効きます。画面に焼き込むと、第7段階で費用を測ったあとに
 * 調整できなくなります。
 */

/** 承認者ID。誰として操作しているかは、常に画面に出す。 */
export function approverId() {
  return localStorage.getItem('approver_id') || '';
}

export function setApproverId(id) {
  localStorage.setItem('approver_id', id.trim());
}

async function json(path, options = {}) {
  const res = await fetch(path, options);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message || `${res.status} ${res.statusText}`);
  }
  return res.json();
}

export const getSettings = () => json('/api/settings');
export const getHistory = () => json('/api/history');
export const getAudit = () => json('/api/audit');

/**
 * 承認待ち一覧。
 *
 * ETag を覚えておき、変わっていなければ本文を受け取りません（304）。
 * 変わっていないときに再描画もしないので、開いたまま放置しても画面が点滅しません。
 */
let queueEtag = null;
export async function getQueue() {
  const res = await fetch('/api/queue', {
    headers: queueEtag ? { 'If-None-Match': queueEtag } : {},
  });
  if (res.status === 304) return null; // 変わっていない
  queueEtag = res.headers.get('etag');
  return res.json();
}

/** 判断を送る。承認者IDが無ければサーバが 401 を返す。 */
export function decide(requestId, verdict) {
  return json(`/api/requests/${requestId}/decision`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'X-Approver-Id': approverId(),
    },
    body: JSON.stringify({ verdict }),
  });
}

/** 閾値シミュレーション。本文は読み直さず、保存済みの内訳だけで再判定される。 */
export function simulate(mediumAt, highAt) {
  return json('/api/simulate', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ medium_at: mediumAt, high_at: highAt }),
  });
}
