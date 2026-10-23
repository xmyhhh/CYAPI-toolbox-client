export type UsageWorkspace = 'codex' | 'claude' | 'opencode';
export interface UsageRecord {
  id: string;
  sessionId: string;
  model: string;
  project: string;
  time: number;
  input: number;
  output: number;
  cachedInput: number;
  cacheWrite: number;
  cacheWrite1h: number;
  reasoningOutput: number;
  reportedCost: number | null;
}
export interface UsageSource {
  path: string;
  format: string;
  files: number;
  cachedFiles: number;
  records: number;
  status: 'available' | 'missing' | 'error';
  notes: string[];
}
export interface UsageReport {
  workspace: UsageWorkspace;
  generatedAt: number;
  records: UsageRecord[];
  sources: UsageSource[];
  duplicates: number;
  partial: boolean;
  notes: string[];
}
// USD per million tokens. Exact model names only: custom gateway aliases require a price.
export interface UsagePrice {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cacheWrite1h: number;
}
export type UsagePrices = Record<string, UsagePrice>;
const price = (
  input: number,
  output: number,
  cacheRead: number,
  cacheWrite = input,
  cacheWrite1h = cacheWrite,
) => ({ input, output, cacheRead, cacheWrite, cacheWrite1h });
// Reference rates, not live quotes or the user's gateway bill. No prefix/fuzzy matching.
export const referencePrices: UsagePrices = {
  'gpt-4o': price(2.5, 10, 1.25),
  'gpt-4o-mini': price(0.15, 0.6, 0.075),
  'gpt-4.1': price(2, 8, 0.5),
  'gpt-4.1-mini': price(0.4, 1.6, 0.1),
  'gpt-5': price(1.25, 10, 0.125),
  'gpt-5-mini': price(0.25, 2, 0.025),
  'gpt-5-nano': price(0.05, 0.4, 0.005),
  'gpt-5.1': price(1.25, 10, 0.125),
  'gpt-5.1-codex': price(1.25, 10, 0.125),
  'claude-sonnet-4-20250514': price(3, 15, 0.3, 3.75, 6),
  'claude-sonnet-4-5-20250929': price(3, 15, 0.3, 3.75, 6),
  'claude-opus-4-1-20250805': price(15, 75, 1.5, 18.75, 30),
};
export const priceFields = ['input', 'output', 'cacheRead', 'cacheWrite', 'cacheWrite1h'] as const;
export function modelPrice(model: string, prices: UsagePrices): UsagePrice | undefined {
  return Object.hasOwn(prices, model)
    ? prices[model]
    : Object.hasOwn(referencePrices, model)
      ? referencePrices[model]
      : undefined;
}
export function validPrice(value: unknown): value is UsagePrice {
  return (
    !!value &&
    typeof value === 'object' &&
    priceFields.every((key) => {
      const n = (value as UsagePrice)[key];
      return typeof n === 'number' && Number.isFinite(n) && n >= 0 && n <= 1_000_000;
    })
  );
}
export function readPrices(raw: string | null): UsagePrices {
  try {
    const data: unknown = JSON.parse(raw ?? '{}');
    if (!data || Array.isArray(data) || typeof data !== 'object') return {};
    return Object.fromEntries(
      Object.entries(data).filter(([key, value]) => key.length <= 160 && validPrice(value)),
    );
  } catch {
    return {};
  }
}
export interface UsageCost {
  usd: number | null;
  kind: 'configured' | 'reported' | 'reference' | 'unknown';
}
export function usageCost(record: UsageRecord, prices: UsagePrices, multiplier = 1): UsageCost {
  const configured = Object.hasOwn(prices, record.model) ? prices[record.model] : undefined;
  if (
    !configured &&
    record.reportedCost !== null &&
    Number.isFinite(record.reportedCost) &&
    record.reportedCost > 0
  ) {
    return { usd: record.reportedCost * multiplier, kind: 'reported' };
  }
  const p =
    configured ??
    (Object.hasOwn(referencePrices, record.model) ? referencePrices[record.model] : undefined);
  if (!p || !validPrice(p)) return { usd: null, kind: 'unknown' };
  const cached = Math.min(record.cachedInput, record.input);
  const write = Math.min(record.cacheWrite, record.input - cached);
  const oneHour = Math.min(record.cacheWrite1h, write);
  return {
    usd:
      (((record.input - cached - write) * p.input +
        record.output * p.output +
        cached * p.cacheRead +
        (write - oneHour) * p.cacheWrite +
        oneHour * p.cacheWrite1h) /
        1_000_000) *
      multiplier,
    kind: configured ? 'configured' : 'reference',
  };
}
export const dayKey = (time: number) => {
  const d = new Date(time);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};
export type UsageRange = 'today' | '7d' | '30d' | 'all' | 'custom';
export function usageBounds(
  range: UsageRange,
  start: string,
  end: string,
  now = new Date(),
): { from: number; to: number; error?: string } {
  if (range === 'all') return { from: 0, to: Infinity };
  const parse = (value: string) => {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return NaN;
    const d = new Date(`${value}T00:00:00`);
    return Number.isFinite(d.getTime()) && dayKey(d.getTime()) === value ? d.getTime() : NaN;
  };
  if (range === 'custom') {
    const from = parse(start);
    const last = parse(end);
    if (!Number.isFinite(from) || !Number.isFinite(last) || from > last)
      return { from: 0, to: 0, error: '请选择有效的起止日期，开始日期不能晚于结束日期。' };
    const next = new Date(last);
    next.setDate(next.getDate() + 1);
    return { from, to: next.getTime() };
  }
  const from = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  from.setDate(from.getDate() - (range === 'today' ? 0 : range === '7d' ? 6 : 29));
  const to = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  return { from: from.getTime(), to: to.getTime() };
}
export interface UsageSummary {
  count: number;
  sessions: number;
  input: number;
  output: number;
  cached: number;
  cacheWrite: number;
  reasoning: number;
  cost: number;
  unpriced: number;
}
export function summarizeUsage(
  records: UsageRecord[],
  prices: UsagePrices,
  multiplier: number,
): UsageSummary {
  const totals = {
    count: records.length,
    sessions: new Set(records.map((r) => r.sessionId)).size,
    input: 0,
    output: 0,
    cached: 0,
    cacheWrite: 0,
    reasoning: 0,
    cost: 0,
    unpriced: 0,
  };
  for (const record of records) {
    totals.input += record.input;
    totals.output += record.output;
    totals.cached += record.cachedInput;
    totals.cacheWrite += record.cacheWrite;
    totals.reasoning += record.reasoningOutput;
    const cost = usageCost(record, prices, multiplier);
    if (cost.usd === null) totals.unpriced++;
    else totals.cost += cost.usd;
  }
  return totals;
}
export function usageTrend(
  records: UsageRecord[],
  prices: UsagePrices,
  multiplier: number,
  from: number,
  to: number,
) {
  const first =
    Number.isFinite(from) && from > 0
      ? from
      : records.reduce<number | undefined>(
          (n, r) => (n === undefined ? r.time : Math.min(n, r.time)),
          undefined,
        );
  const last = Number.isFinite(to)
    ? to - 1
    : records.reduce<number | undefined>(
        (n, r) => (n === undefined ? r.time : Math.max(n, r.time)),
        undefined,
      );
  if (first === undefined || last === undefined) return [];
  const calendarDay = (t: number) => Date.parse(`${dayKey(t)}T00:00:00Z`) / 86400000;
  const begin = calendarDay(first);
  const end = calendarDay(last);
  if (!Number.isFinite(begin) || !Number.isFinite(end) || begin > end) return [];
  const step = Math.max(1, Math.ceil((end - begin + 1) / 31));
  const buckets = Array.from(
    { length: Math.min(31, Math.ceil((end - begin + 1) / step)) },
    (_, i) => {
      const date = (n: number) => new Date(n * 86400000).toISOString().slice(0, 10);
      return {
        start: date(begin + i * step),
        end: date(Math.min(end, begin + (i + 1) * step - 1)),
        input: 0,
        output: 0,
        cost: 0,
        count: 0,
        unpriced: 0,
      };
    },
  );
  for (const record of records) {
    const bucket = buckets[Math.floor((calendarDay(record.time) - begin) / step)];
    if (!bucket) continue;
    bucket.input += record.input;
    bucket.output += record.output;
    bucket.count++;
    const cost = usageCost(record, prices, multiplier);
    if (cost.usd === null) bucket.unpriced++;
    else bucket.cost += cost.usd;
  }
  return buckets;
}
export function usageCsv(records: UsageRecord[], prices: UsagePrices, multiplier: number): string {
  const cell = (value: string | number) => {
    const raw = String(value);
    const safe = /^[\s]*[=+\-@\t\r]/.test(raw) ? `'${raw}` : raw;
    return `"${safe.replaceAll('"', '""')}"`;
  };
  const rows = [
    [
      '时间',
      '模型',
      '会话',
      '项目',
      '输入 Token（含缓存）',
      '输出 Token（含推理）',
      '缓存读取',
      '缓存写入',
      '其中 1h 写入',
      '其中推理输出',
      '估算 USD',
      '费用来源',
    ],
    ...records.map((r) => {
      const cost = usageCost(r, prices, multiplier);
      return [
        new Date(r.time).toISOString(),
        r.model,
        r.sessionId,
        r.project,
        r.input,
        r.output,
        r.cachedInput,
        r.cacheWrite,
        r.cacheWrite1h,
        r.reasoningOutput,
        cost.usd === null ? '' : cost.usd.toFixed(8),
        cost.kind,
      ];
    }),
  ];
  return '\uFEFF' + rows.map((row) => row.map(cell).join(',')).join('\r\n');
}
