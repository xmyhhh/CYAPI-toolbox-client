import test from 'node:test';
import assert from 'node:assert/strict';
import { updateProgressText } from '../src/lib/update-progress.ts';
import type { UpdateProgress } from '../src/lib/update-progress.ts';

const progress: UpdateProgress = {
  operationId: 'test-update',
  stage: 'download',
  downloaded: 1024 * 1024,
  total: 4 * 1024 * 1024,
};

test('update progress distinguishes connecting, transfer, verification and completion', () => {
  assert.equal(updateProgressText({ ...progress, stage: 'manifest' }), '正在获取更新清单');
  assert.match(updateProgressText({ ...progress, downloaded: 0 }), /正在连接下载服务器/);
  assert.match(updateProgressText({ ...progress, total: 0 }), /官网下载/);
  assert.equal(updateProgressText(progress), '正在下载更新安装包：25%（1.0 / 4.0 MB）');
  assert.match(updateProgressText({ ...progress, stage: 'verify' }), /正在校验/);
  assert.match(updateProgressText({ ...progress, stage: 'verified' }), /校验通过/);
});

test('update progress resets on retry and caps displayed percentages', () => {
  assert.match(updateProgressText({ ...progress, downloaded: progress.total }), /100%/);
  assert.match(updateProgressText({ ...progress, downloaded: progress.total + 10 }), /100%/);
  assert.match(updateProgressText({ ...progress, downloaded: 0 }), /正在连接/);
});
