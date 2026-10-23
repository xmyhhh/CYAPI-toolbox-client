import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './app/App';
import './app/theme.css';
import './app/native.css';
import { invoke, isTauri } from '@tauri-apps/api/core';

if (isTauri()) {
  const report = (message: string) => {
    void invoke('log_frontend_error', { message }).catch(() => {});
  };
  window.addEventListener('error', (event) => report(event.message || '前端运行错误'));
  window.addEventListener('unhandledrejection', (event) =>
    report(event.reason instanceof Error ? event.reason.message : '未处理的异步任务失败'),
  );
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
