import { emit } from '@tauri-apps/api/event';
import { nextTick } from 'vue';
import * as agent from '../src/composables/useAgent';

export async function runPermissionChecks() {
  const ui = agent.useAgentState();
  const tick = async () => { await nextTick(); await new Promise(resolve => { const channel = new MessageChannel(); channel.port1.onmessage = () => { channel.port1.close(); channel.port2.close(); resolve(); }; channel.port2.postMessage(null); }); await nextTick(); };
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const original = window.__TAURI_INTERNALS__.invoke;
  let release;
  let rejectDelete = true;
  const replies = [];
  window.__TAURI_INTERNALS__.invoke = async (command, args, options) => {
    if (command === 'session_delete') {
      await new Promise(resolve => { release = resolve; });
      if (rejectDelete) throw new Error('测试：删除写入失败');
    }
    if (command === 'permission_respond') { replies.push(args); return; }
    return original(command, args, options);
  };
  try {
    const a = await agent.startSession();
    const b = await agent.startSession(); await tick();
    const deletion = agent.deleteSession(a).then(() => false, () => true);
    const request = (id, session) => emit('acp-event', { type: 'permission_request', session_id: session, request_id: id, tool_call: { title: `测试审批${id}` }, options: [{ optionId: `opt-${id}`, name: '允许一次', kind: 'allow_once' }] });
    await request(31, a); await request(32, b); await tick();
    assert(ui.permission.value?.requestId === 32, '删除中甲的权限不能挡住乙的真实审批');
    assert(agent.pendingPermissionCount() === 1, '隐藏审批不能计入可操作审批数');
    agent.resolvePermission('opt-32'); await tick();
    assert(replies.length === 1 && replies[0].requestId === 32, 'resolve不能shift掉隐藏的甲请求');
    release(); assert(await deletion, '删除失败应向调用方报告'); await tick();
    assert(ui.permission.value?.requestId === 31, '删除失败后甲未失效的请求必须恢复可见');
    assert(ui.sessionId.value === b, '删除失败不得抢走乙的当前视图');
    rejectDelete = false;
    const successful = agent.deleteSession(a); await tick(); release(); await successful; await tick();
    assert(!ui.permission.value && agent.pendingPermissionCount() === 0, '成功删除后目标旧权限必须清除');
    await request(33, a); await tick();
    assert(!ui.permission.value, '删除后的迟到审批不得复活');
    return { result: 'PASS', checks: ['删除中权限隐藏', '其他会话审批不被错答', '删除失败审批恢复', '成功删除及迟到审批清理'] };
  } finally { window.__TAURI_INTERNALS__.invoke = original; }
}

export async function runPermissionFailureRace() {
  const ui = agent.useAgentState();
  const tick = async () => { await nextTick(); await new Promise(resolve => { const c = new MessageChannel(); c.port1.onmessage = () => { c.port1.close(); c.port2.close(); resolve(); }; c.port2.postMessage(null); }); await nextTick(); };
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  const original = window.__TAURI_INTERNALS__.invoke;
  let finishReply;
  let finishDelete;
  let failDelete = true;
  window.__TAURI_INTERNALS__.invoke = async (cmd, args, options) => {
    if (cmd === 'permission_respond') { await new Promise(resolve => { finishReply = resolve; }); throw new Error('测试：应答网络错误'); }
    if (cmd === 'session_delete' && failDelete) { await new Promise(resolve => { finishDelete = resolve; }); throw new Error('测试：删除失败'); }
    return original(cmd, args, options);
  };
  try {
    const a = await agent.startSession();
    const b = await agent.startSession(); await tick();
    await emit('acp-event', { type: 'permission_request', session_id: a, request_id: 41, tool_call: { title: '仍有效的审批' }, options: [{ optionId: 'once', name: '允许一次', kind: 'allow_once' }] });
    await tick(); agent.resolvePermission('once'); await tick();
    const deletion = agent.deleteSession(a).catch(() => {}); await tick();
    finishReply(); await tick();
    assert(!ui.permission.value, '删除中应答失败的请求应暂时隐藏');
    finishDelete(); await deletion; await tick();
    assert(ui.permission.value?.requestId === 41, '删除与应答均失败时，仍有效的审批不能丢失');
    assert(ui.sessionId.value === b, '失败收尾不得抢走另一任务');
    failDelete = false; await agent.deleteSession(a); await tick();
    assert(!ui.deletingSessions.value.includes(a), '成功删除后必须清除删除中标记');
    assert(ui.removedSessions.value.includes(a), '必须保留已删除防迟到标记');
    assert(!ui.permission.value, '成功删除后不保留审批');
    return { result: 'PASS', checks: ['应答失败与删除失败竞态不丢请求', '成功清删除中状态', '已删除水位保留'] };
  } finally { window.__TAURI_INTERNALS__.invoke = original; }
}
