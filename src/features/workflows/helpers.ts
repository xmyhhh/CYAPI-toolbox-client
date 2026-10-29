import type {
  WorkflowCategory,
  WorkflowDisplay,
  WorkflowStatus,
  WorkflowTask,
  WorkflowTemplate,
} from './types';

const asset = (name: string) => new URL(`../../assets/workflows/${name}`, import.meta.url).href;

export const PRODUCT_COVER = asset('product.svg');
export const IDEA_COVER = asset('idea.svg');

export interface Feature {
  label: string;
  icon: string;
  input: string;
  output: string;
  steps: string[];
  note: string;
  price?: WorkflowDisplay['price'];
}

const FALLBACK: Feature = {
  label: '',
  icon: 'workflow',
  input: '—',
  output: '—',
  steps: [],
  note: '',
};

export function feature(item: { display?: WorkflowDisplay }): Feature {
  return { ...FALLBACK, ...(item.display || {}) };
}

export const priceLabel = (item: { display?: WorkflowDisplay }, billed: boolean) => {
  if (!billed) return 'Mock · 不计费';
  const price = feature(item).price;
  return price ? `${Number(price.amount).toFixed(2)} ${price.currency} / ${price.unit}` : '以运行前报价为准';
};

export function templateCover(item: { cover?: string }) {
  return item.cover === 'product' ? PRODUCT_COVER : IDEA_COVER;
}

export function libraryItems(
  categories: WorkflowCategory[],
  templates: WorkflowTemplate[],
  filter: string,
  query = '',
) {
  const category = categories.find((item) => item.id === filter);
  const items = !category
    ? []
    : category.scope === 'all'
      ? templates
      : category.scope === 'personal'
        ? templates.filter((item) => item.source === 'personal')
        : (category.templateIds || [])
            .map((id) => templates.find((item) => item.source === 'server' && item.id === id))
            .filter((item): item is WorkflowTemplate => Boolean(item));
  const search = query.trim().toLowerCase();
  if (!search) return items;
  return items.filter((item) =>
    `${item.name} ${item.summary} ${item.category} ${feature(item).label}`
      .toLowerCase()
      .includes(search),
  );
}

export const activeStatuses = new Set<WorkflowStatus>([
  'reserving',
  'submitting',
  'queued',
  'running',
  'collecting',
  'settling',
  'releasing',
]);

export const isActive = (status: WorkflowStatus) => activeStatuses.has(status);

export const statusLabels: Record<string, string> = {
  reserving: '预留额度',
  settling: '扣费中',
  releasing: '撤销预留',
  reservation_unknown: '预留待核对',
  billing_unknown: '账务待核对',
  submitting: '提交中',
  queued: '排队中',
  running: '生成中',
  collecting: '保存结果',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已取消',
  interrupted: '已中断',
  unknown: '状态待核对',
};

export const stepLabels: Record<string, string> = {
  waiting: '等待前置步骤',
  running: '执行中',
  succeeded: '成功',
  failed: '失败',
  skipped: '前置失败，已跳过',
  cancelled: '已取消',
};

export const billingLabels: Record<string, string> = {
  reserved: '已预留',
  settled: '已扣费',
  released: '未扣款',
};

export const statusLabel = (status: string) => statusLabels[status] || status;

export function formatDate(timestamp?: number) {
  if (!timestamp) return '—';
  return new Date(timestamp).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  });
}

export const outputLabel = (kind?: string) =>
  ({ image: '图片', video: '视频', mesh: '3D 网格', text: '文本' })[kind || 'image'] || '产物';

export const countActive = (tasks: WorkflowTask[]) =>
  tasks.filter((task) => isActive(task.status)).length;

export interface Example {
  title: string;
  input: string;
  image: string;
  output: string;
  kind: 'image' | 'video' | 'mesh' | 'text' | 'composite';
}

export const examples: Record<string, Example> = {
  'product-studio': {
    title: '护肤品场景图',
    input: '参考图：一瓶绿色护肤品。描述：浅绿色背景，圆形展台，柔和侧光与植物阴影。',
    image: PRODUCT_COVER,
    output: '产品场景图示意',
    kind: 'image',
  },
  'image-idea': {
    title: '梦幻星球插画',
    input: '一颗粉色星球悬浮在蓝紫色沙丘上方，柔和渐变，极简插画。',
    image: IDEA_COVER,
    output: '创意插画示意',
    kind: 'image',
  },
  'mock-video': {
    title: '产品旋转短片',
    input: '蓝绿色展示盒，在明亮的工作室中缓慢旋转，固定镜头。',
    image: asset('video.svg'),
    output: '可播放的短片演示',
    kind: 'video',
  },
  'mock-mesh': {
    title: '立方体网格',
    input: '一个简单的立方体，保留清晰的棱角，用于检查 3D 网格与旋转预览。',
    image: asset('mesh.svg'),
    output: 'OBJ · 8 顶点 / 12 三角面',
    kind: 'mesh',
  },
  'mock-transcribe': {
    title: '语音创意转文字',
    input: '示例语音内容：“一只蓝色产品展示盒，在明亮的工作室中缓慢旋转。”',
    image: asset('voice.svg'),
    output: '可阅读、下载的 TXT 文本',
    kind: 'text',
  },
  'mock-composite': {
    title: '一句创意，多种产物',
    input: '语音描述一个产品展示场景，转写后生成图片，再并行生成视频与 3D 网格。',
    image: asset('composite.svg'),
    output: '文字 → 图片 → 视频 + 3D',
    kind: 'composite',
  },
};

export const exampleFor = (item: WorkflowTemplate) =>
  item.source === 'server' ? examples[item.id] : undefined;

export const DEMO_VIDEO = asset('video.webm');
export const DEMO_VIDEO_POSTER = asset('video.svg');
export const DEMO_MESH = asset('cube.obj');
export const DEMO_TRANSCRIPT = '一只蓝色产品展示盒，在明亮的工作室中缓慢旋转。';

export function safeFilename(name: string, fallback = 'workflow') {
  return (
    name
      .replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_')
      .trim()
      .slice(0, 60) || fallback
  );
}

/** The graph is JSON; only the transcribe nodes add an audio field. */
export function needsAudio(item: { apiGraph?: Record<string, unknown> }) {
  return Object.values(item.apiGraph || {}).some((node) => {
    const type = (node as { class_type?: unknown })?.class_type;
    return typeof type === 'string' && type.includes('Transcribe');
  });
}
