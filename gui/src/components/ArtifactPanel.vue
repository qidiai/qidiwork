<script setup lang="ts">
// 右区:产物面板。P1 接入 manifest(fsnotify 实时刷新)后展示产物卡片;
// 点击卡片在中区开预览 Tab(M1 仅空状态)。
</script>

<template>
  <aside class="artifact-panel">
    <div class="panel-title">产物面板</div>
    <div class="panel-body">
      <p class="empty-hint">任务产出的交付文件会出现在这里。</p>
      <ul class="hint-list">
        <li>点击卡片可在中间预览(docx / xlsx / PDF / 图片)</li>
        <li>「打开」用系统默认程序(WPS / Word)编辑</li>
        <li>产物由办公技能自动登记,无需手动上传</li>
      </ul>
    </div>
  </aside>
</template>

<style scoped>
.artifact-panel {
  background: var(--bg-panel);
  border-left: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.panel-title {
  padding: 12px 14px;
  font-size: var(--font-size-sm);
  font-weight: 600;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border);
}

.panel-body {
  flex: 1;
  overflow-y: auto;
  padding: 14px;
}

.hint-list {
  margin: 8px 0 0;
  padding-left: 18px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.9;
}
</style>
