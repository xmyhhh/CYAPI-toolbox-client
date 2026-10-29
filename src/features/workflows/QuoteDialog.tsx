import { useState } from 'react';
import { LoaderCircle, Play, ShieldCheck } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { WorkflowError } from './api';
import type { WorkflowQuote } from './types';

export function QuoteDialog({
  quote,
  draftName,
  keyName,
  onConfirm,
  onClose,
}: {
  quote: WorkflowQuote;
  draftName: string;
  keyName: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [stale, setStale] = useState(false);
  const billed = quote.mode === 'local-billing';

  const confirm = async () => {
    if (busy || stale) return;
    setBusy(true);
    setError('');
    try {
      await onConfirm();
    } catch (cause) {
      if (cause instanceof WorkflowError && cause.status === 409) {
        setStale(true);
        setError('报价或草稿已变化。未自动重试，请关闭后重新打开草稿并再次确认。');
      } else {
        setError(cause instanceof Error ? cause.message : '任务提交失败，请稍后重试');
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title="确认 Mock 任务"
      onClose={onClose}
      className="wfx-dialog"
      busy={busy}
      dismissWhileBusy
    >
      <div className="wfx-dialog-body">
        <p className="wfx-eyebrow">本地模拟 · 非 AI 生成</p>
        <p className="wfx-quote-name">{draftName}</p>
        {quote.items && quote.items.length > 0 ? (
          <dl className="wfx-info wfx-quote-items">
            {quote.items.map((item) => (
              <div key={item.id}>
                <dt>{item.title}</dt>
                <dd>{Number(item.amount).toFixed(2)} USD</dd>
              </div>
            ))}
            <div className="total">
              <dt>合计</dt>
              <dd>
                {Number(quote.amount).toFixed(2)} {quote.unit}
              </dd>
            </div>
          </dl>
        ) : (
          <dl className="wfx-info wfx-quote-items">
            <div className="total">
              <dt>合计</dt>
              <dd>
                {Number(quote.amount)} {quote.unit}
              </dd>
            </div>
          </dl>
        )}
        <p className="wfx-quote-note">
          {billed
            ? '任务成功后才会按计费单位扣费；明确失败或取消不扣费。状态未知时保留待核对。'
            : '不计费，仅测试确认流程。'}
        </p>
        <p className="wfx-quote-account">
          <ShieldCheck size={14} aria-hidden="true" />
          本次使用：{keyName}。费用与记录归属此 Key。
        </p>
        {error && (
          <p className="wfx-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="n-dialog-footer">
        <span>{billed ? '测试余额' : 'Mock 不计费'}</span>
        <button type="button" className="wfx-button small" onClick={onClose} disabled={busy}>
          {stale ? '关闭' : '取消'}
        </button>
        <button
          type="button"
          className="wfx-button primary"
          onClick={() => void confirm()}
          disabled={busy || stale}
        >
          {busy ? <LoaderCircle className="n-spin" size={15} /> : <Play size={15} />}
          确认并启动
        </button>
      </div>
    </Modal>
  );
}
