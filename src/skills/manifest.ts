import { parse } from 'yaml';
import type { Skill } from './types';

export const MAX_MANIFEST_BYTES = 256 * 1024;

export function parseSkillManifest(markdown: string, fileCount = 1): Skill {
  if (new TextEncoder().encode(markdown).length > MAX_MANIFEST_BYTES) {
    throw new Error('SKILL.md 超过 256 KB，请使用更小的说明文件。');
  }
  const normalized = markdown.replace(/^\uFEFF/, '').replace(/\r\n/g, '\n');
  const frontmatter = /^---[ \t]*\n([\s\S]*?)\n---[ \t]*(?:\n|$)/.exec(normalized);
  if (!frontmatter)
    throw new Error('缺少 YAML 元数据。请在文件开头用 --- 包含 name 和 description。');
  let metadata: unknown;
  try {
    metadata = parse(frontmatter[1], { maxAliasCount: 20, uniqueKeys: true });
  } catch {
    throw new Error('YAML 元数据格式不正确，请检查缩进、引号或重复字段。');
  }
  if (!metadata || typeof metadata !== 'object' || Array.isArray(metadata)) {
    throw new Error('元数据必须包含 name 和 description 字段。');
  }
  const { name, description } = metadata as Record<string, unknown>;
  if (typeof name !== 'string' || name.length > 64 || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(name)) {
    throw new Error('name 只能包含小写字母、数字和单个连字符，最多 64 个字符。');
  }
  if (typeof description !== 'string' || !description.trim() || description.length > 1024) {
    throw new Error('description 必须是 1–1024 个字符的文字。');
  }
  return {
    id: `local-${name}`,
    name,
    title: name,
    description: description.trim(),
    category: '工作流程',
    icon: 'blocks',
    author: '本地导入',
    version: '本地版本',
    source: 'local',
    fileCount,
    markdown: normalized,
  };
}

export function manifestBody(markdown: string) {
  return markdown.replace(/^---[ \t]*\n[\s\S]*?\n---[ \t]*(?:\n|$)/, '');
}
