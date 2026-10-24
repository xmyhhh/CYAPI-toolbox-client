export interface UpdateProgress {
  operationId: string;
  stage: 'manifest' | 'download' | 'verify' | 'verified';
  downloaded: number;
  total: number;
}

/** Keep connection/retry waits distinguishable from a running transfer. */
export function updateProgressText(progress: UpdateProgress): string {
  switch (progress.stage) {
    case 'manifest':
      return '正在获取更新清单';
    case 'verify':
      return '下载完成，正在校验更新包';
    case 'verified':
      return '更新包校验通过，正在准备安装';
    case 'download': {
      if (progress.downloaded <= 0 || progress.total <= 0)
        return '正在连接下载服务器（线路失败时自动重试，可改用官网下载）';
      const downloaded = Math.min(progress.downloaded, progress.total);
      const percent = Math.min(100, Math.floor((downloaded / progress.total) * 100));
      const size = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
      return `正在下载更新安装包：${percent}%（${size(downloaded)} / ${size(progress.total)} MB）`;
    }
  }
}
