import { useEffect, useState } from 'react';
import type { ConfigValue, IntegrationAction, IntegrationEntry } from '../../lib/api/native';

function at(value: unknown, path: string[] = []): unknown {
  return path.reduce<unknown>(
    (v, key) => (v && typeof v === 'object' ? (v as Record<string, unknown>)[key] : undefined),
    value,
  );
}
function display(value: unknown) {
  return typeof value === 'string' || typeof value === 'number' ? String(value) : '';
}

export function ActionResult({ action, value }: { action: IntegrationAction; value: unknown }) {
  const rows = at(value, action.result?.listPath);
  const columns = action.result?.columns;
  const outputPath = action.result?.path ? display(at(value, action.result.path)) : '';
  return (
    <div className="n-draft-result" role="status">
      <span>{action.result?.message || display(at(value, ['message'])) || '操作完成'}</span>
      {outputPath && <code>{outputPath}</code>}
      {Array.isArray(rows) && columns && (
        <div className="n-integration-results">
          {rows.length ? (
            <table>
              <thead>
                <tr>
                  {columns.map((column) => (
                    <th key={column.label}>{column.label}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.slice(0, 100).map((row, i) => (
                  <tr key={i}>
                    {columns.map((column) => (
                      <td key={column.label}>{display(at(row, column.path))}</td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <span>暂无结果</span>
          )}
        </div>
      )}
    </div>
  );
}
export function ActionForm({
  action,
  entry,
  disabled,
  run,
}: {
  action: IntegrationAction;
  entry?: IntegrationEntry;
  disabled: boolean;
  run: (action: IntegrationAction, values: Record<string, ConfigValue>) => void;
}) {
  const [values, setValues] = useState<Record<string, ConfigValue>>({});
  useEffect(() => {
    setValues(
      Object.fromEntries(
        action.fields.map((field) => [
          field.id,
          field.type === 'secret' ? '' : (entry?.health.values?.[field.id] ?? field.default),
        ]),
      ),
    );
  }, [action, entry?.health.values]);
  return (
    <form
      className="n-draft-settings"
      onSubmit={(event) => {
        event.preventDefault();
        run(action, values);
      }}
    >
      {action.fields.map((field) => (
        <label key={field.id}>
          <span>{field.label}</span>
          {field.type === 'boolean' ? (
            <input
              type="checkbox"
              checked={!!values[field.id]}
              disabled={disabled}
              onChange={(event) => setValues({ ...values, [field.id]: event.target.checked })}
            />
          ) : (
            <input
              type={
                field.type === 'secret' ? 'password' : field.type === 'number' ? 'number' : 'text'
              }
              value={String(values[field.id] ?? '')}
              placeholder={field.placeholder}
              disabled={disabled}
              onChange={(event) =>
                setValues({
                  ...values,
                  [field.id]:
                    field.type === 'number' ? Number(event.target.value) : event.target.value,
                })
              }
            />
          )}
        </label>
      ))}
      <div>
        <button className="n-button" disabled={disabled}>
          {action.label}
        </button>
      </div>
    </form>
  );
}
