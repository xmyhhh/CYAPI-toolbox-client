import test from 'node:test';
import assert from 'node:assert/strict';
import {
  dayKey,
  modelPrice,
  readPrices,
  summarizeUsage,
  usageBounds,
  usageCost,
  usageCsv,
  usageTrend,
  validPrice,
} from '../src/lib/usage.ts';
import type { UsagePrice, UsageRecord } from '../src/lib/usage.ts';

const p: UsagePrice = { input: 2, output: 10, cacheRead: 0.5, cacheWrite: 2.5, cacheWrite1h: 4 };
const row = (overrides: Partial<UsageRecord> = {}): UsageRecord => ({
  id: 'r',
  sessionId: 's',
  model: 'custom-model',
  project: 'F:/project',
  time: new Date(2026, 9, 20, 12).getTime(),
  input: 1000,
  output: 200,
  cachedInput: 600,
  cacheWrite: 100,
  cacheWrite1h: 40,
  reasoningOutput: 100,
  reportedCost: null,
  ...overrides,
});
const close = (actual: number | null, expected: number) =>
  assert.ok(actual !== null && Math.abs(actual - expected) < 1e-10, `${actual} != ${expected}`);

test('cost charges cache subsets and reasoning exactly once, including 1h writes and multipliers', () => {
  close(
    usageCost(row(), { 'custom-model': p }, 2).usd,
    ((300 * 2 + 200 * 10 + 600 * 0.5 + 60 * 2.5 + 40 * 4) / 1e6) * 2,
  );
  const clamped = row({ cachedInput: 2000, cacheWrite: 2000, cacheWrite1h: 5000 });
  close(usageCost(clamped, { 'custom-model': p }).usd, (1000 * 0.5 + 200 * 10) / 1e6);
});
test('unknown costs stay unknown; explicit zero rates are valid; aliases do not inherit prices', () => {
  assert.equal(usageCost(row(), {}).usd, null);
  assert.equal(usageCost(row({ model: 'gpt-5-custom' }), {}).usd, null);
  assert.equal(usageCost(row({ reportedCost: 0 }), {}).usd, null);
  assert.equal(
    usageCost(row(), {
      'custom-model': { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cacheWrite1h: 0 },
    }).usd,
    0,
  );
  assert.equal(usageCost(row({ model: 'gpt-5' }), {}).kind, 'reference');
});
test('custom prices override recorded costs, which override reference rates', () => {
  assert.equal(usageCost(row({ model: 'gpt-5', reportedCost: 0.3 }), {}).kind, 'reported');
  close(usageCost(row({ model: 'gpt-5', reportedCost: 0.3 }), {}, 2).usd, 0.6);
  assert.equal(usageCost(row({ reportedCost: 99 }), { 'custom-model': p }).kind, 'configured');
  assert.equal(usageCost(row({ reportedCost: Infinity }), {}).usd, null);
});
test('stored prices reject malformed, negative, missing and prototype values', () => {
  assert.deepEqual(readPrices('{bad'), {});
  assert.deepEqual(readPrices('[]'), {});
  assert.deepEqual(
    readPrices(
      JSON.stringify({
        good: p,
        bad: { ...p, input: -1 },
        missing: { input: 2 },
        badType: { ...p, input: '2' },
      }),
    ),
    { good: p },
  );
  assert.equal(validPrice({ ...p, input: NaN }), false);
  assert.equal(modelPrice('__proto__', {}), undefined);
  assert.equal(usageCost(row({ model: 'constructor' }), {}).usd, null);
});
test('local date ranges include today and the entire end day and reject invalid dates', () => {
  const today = new Date(2026, 9, 20, 14, 30);
  const week = usageBounds('7d', '', '', today);
  assert.equal(dayKey(week.from), '2026-10-14');
  assert.equal(dayKey(week.to), '2026-10-21');
  const custom = usageBounds('custom', '2026-10-01', '2026-10-20', today);
  assert.equal(dayKey(custom.to), '2026-10-21');
  assert.ok(usageBounds('custom', '2026-02-31', '2026-03-10').error);
  assert.ok(usageBounds('custom', '2026-10-21', '2026-10-20').error);
  assert.ok(usageBounds('custom', '', '').error);
  assert.equal(usageBounds('all', '', '').to, Infinity);
});
test('summary counts distinct sessions and retains missing prices separately from known subtotal', () => {
  const result = summarizeUsage(
    [row(), row({ id: 'r2', sessionId: 's2', model: 'unpriced' })],
    { 'custom-model': p },
    1,
  );
  assert.equal(result.count, 2);
  assert.equal(result.sessions, 2);
  assert.equal(result.input, 2000);
  assert.equal(result.output, 400);
  assert.equal(result.cached, 1200);
  assert.equal(result.unpriced, 1);
  close(result.cost, usageCost(row(), { 'custom-model': p }).usd!);
});
test('trend fills empty calendar dates and buckets long ranges without losing usage', () => {
  const bounds = usageBounds('custom', '2026-10-18', '2026-10-20');
  const rows = [row(), row({ id: 'earlier', time: new Date(2026, 9, 18, 9).getTime() })];
  const trend = usageTrend(rows, {}, 1, bounds.from, bounds.to);
  assert.equal(trend.length, 3);
  assert.equal(trend[1].count, 0);
  assert.equal(trend[2].count, 1);
  const year = usageBounds('custom', '2026-01-01', '2026-12-31');
  const buckets = usageTrend(rows, {}, 1, year.from, year.to);
  assert.ok(buckets.length <= 31);
  assert.equal(
    buckets.reduce((n, b) => n + b.count, 0),
    2,
  );
  assert.equal(usageTrend(rows.reverse(), {}, 1, 0, Infinity).length, 3);
  assert.deepEqual(usageTrend([], {}, 1, 0, 0), []);
});
test('CSV escapes formulas and quotes, preserves missing costs and exports normalized totals', () => {
  const csv = usageCsv([row({ model: '=HYPERLINK("bad")', project: '@project' })], {}, 1);
  assert.ok(csv.startsWith('\uFEFF'));
  assert.ok(csv.includes(`"'=HYPERLINK(""bad"")"`));
  assert.ok(csv.includes(`"'@project"`));
  assert.ok(csv.endsWith('"","unknown"'));
  assert.ok(csv.includes('"1000","200","600","100","40","100"'));
});
