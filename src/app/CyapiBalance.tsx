import { useEffect, useState } from 'react';
import { LoaderCircle, RefreshCw, Wallet } from 'lucide-react';
import { nativeAvailable } from '../lib/api/native';
import { readCyapiBalance } from '../lib/api/cyapiBalance';
import type { CyapiBalancePlan } from '../lib/api/cyapiBalance';
import './cyapi-balance.css';

interface Props {
  baseUrl: string;
  apiKey: string;
  disabled: boolean;
}

const amount = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 6 });

export function CyapiBalance({ baseUrl, apiKey, disabled }: Props) {
  const [plans, setPlans] = useState<CyapiBalancePlan[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [updatedAt, setUpdatedAt] = useState<Date>();
  const [refresh, setRefresh] = useState(0);
  const url = baseUrl.trim();
  const key = apiKey.trim();
  const canQuery = nativeAvailable && !disabled && !!url && !!key;

  useEffect(() => {
    let active = true;
    setPlans([]);
    setError('');
    setUpdatedAt(undefined);
    setLoading(canQuery);
    if (!canQuery) return;

    // Wait for typing to settle, and discard responses for an old key or route.
    const timer = window.setTimeout(() => {
      void readCyapiBalance(url, key)
        .then((next) => {
          if (!active) return;
          setPlans(next);
          setUpdatedAt(new Date());
        })
        .catch((error: unknown) => {
          if (active) setError(error instanceof Error ? error.message : String(error));
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    }, 600);

    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [url, key, canQuery, refresh]);

  return (
    <section className="n-cyapi-balance" aria-label="CYAPI 余额查询" aria-busy={loading}>
      <div className="n-cyapi-balance-heading">
        <h3>
          <Wallet size={16} aria-hidden="true" />
          余额
        </h3>
        <button
          type="button"
          className="n-button"
          disabled={!canQuery || loading}
          onClick={() => setRefresh((value) => value + 1)}
        >
          {loading ? (
            <LoaderCircle size={14} className="n-spin" aria-hidden="true" />
          ) : (
            <RefreshCw size={14} aria-hidden="true" />
          )}
          {loading ? '正在查询' : '刷新余额'}
        </button>
      </div>
      {error ? (
        <p className="n-config-note n-error-text" role="alert">
          {error}
        </p>
      ) : (
        <p className="n-cyapi-balance-status" role="status" aria-live="polite">
          {!nativeAvailable
            ? '请在桌面版查询余额。'
            : disabled
              ? '配置就绪后自动查询余额。'
              : !key
                ? '填写 API Key 后自动查询余额。'
                : loading
                  ? '正在查询余额…'
                  : updatedAt
                    ? `更新于 ${updatedAt.toLocaleTimeString('zh-CN')}`
                    : '选择线路后自动查询余额。'}
        </p>
      )}
      {plans.length > 0 && (
        <div className="n-cyapi-balance-plans">
          {plans.map((plan, index) => (
            <article className="n-cyapi-balance-plan" key={index}>
              <div className="n-cyapi-balance-plan-heading">
                <h4>{plan.planName}</h4>
                <span
                  className={`n-pill ${plan.isValid ? 'n-cyapi-balance-valid' : 'n-error-text'}`}
                >
                  {plan.isValid ? '有效' : '已失效'}
                </span>
              </div>
              <p className="n-cyapi-balance-remaining">
                <span>剩余</span>
                <strong>
                  {plan.remaining === undefined ? '未知' : amount.format(plan.remaining)}
                </strong>
                <span>{plan.unit}</span>
              </p>
              {(plan.total !== undefined || plan.used !== undefined) && (
                <dl className="n-cyapi-balance-details">
                  {plan.total !== undefined && (
                    <div>
                      <dt>总额度</dt>
                      <dd>
                        {amount.format(plan.total)} {plan.unit}
                      </dd>
                    </div>
                  )}
                  {plan.used !== undefined && (
                    <div>
                      <dt>已使用</dt>
                      <dd>
                        {amount.format(plan.used)} {plan.unit}
                      </dd>
                    </div>
                  )}
                </dl>
              )}
              {plan.extra && <p className="n-cyapi-balance-reset">{plan.extra}</p>}
            </article>
          ))}
        </div>
      )}
    </section>
  );
}
