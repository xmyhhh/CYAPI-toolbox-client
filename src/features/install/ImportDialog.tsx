import { useRef, useState } from 'react';
import { AlertCircle, ArrowRight, FileCode2, FolderOpen, Upload } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { MAX_MANIFEST_BYTES, parseSkillManifest } from '../../skills/manifest';
import type { Skill } from '../../skills/types';

const placeholder =
  '---\nname: my-first-skill\ndescription: 描述这个 Skill 的能力与使用场景。\n---\n\n# My First Skill\n\n在这里编写 Skill 指令。';

export function ImportDialog({
  onClose,
  onImport,
}: {
  onClose: () => void;
  onImport: (skill: Skill) => void;
}) {
  const [tab, setTab] = useState<'folder' | 'text'>('folder');
  const [text, setText] = useState('');
  const [selected, setSelected] = useState<{ skill: Skill; label: string }>();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const folderInput = useRef<HTMLInputElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);

  async function readFiles(files: FileList | null) {
    if (!files?.length) return;
    setError('');
    setSelected(undefined);
    setBusy(true);
    try {
      const manifests = Array.from(files).filter((file) => file.name.toLowerCase() === 'skill.md');
      if (manifests.length === 0)
        throw new Error('没有找到 SKILL.md，请选择包含该文件的 Skill 目录。');
      if (manifests.length > 1) throw new Error('检测到多个 SKILL.md，请选择单个 Skill 的目录。');
      const manifest = manifests[0];
      if (manifest.size > MAX_MANIFEST_BYTES) throw new Error('SKILL.md 超过 256 KB，无法导入。');
      const skill = parseSkillManifest(await manifest.text(), files.length);
      setSelected({ skill, label: manifest.webkitRelativePath || manifest.name });
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
      if (folderInput.current) folderInput.current.value = '';
      if (fileInput.current) fileInput.current.value = '';
    }
  }

  function submit() {
    setError('');
    try {
      const skill = tab === 'text' ? parseSkillManifest(text) : selected?.skill;
      if (!skill) throw new Error('请先选择包含 SKILL.md 的目录或文件。');
      onImport(skill);
    } catch (reason) {
      setError((reason as Error).message);
    }
  }

  return (
    <Modal title="导入本地 Skill" onClose={onClose} busy={busy}>
      <div className="modal-body import-body">
        <p className="muted">从本机读取 Skill 元数据，添加到你的能力库。</p>
        <div className="segmented">
          <button
            className={tab === 'folder' ? 'active' : ''}
            onClick={() => {
              setTab('folder');
              setError('');
            }}
          >
            <FolderOpen size={16} />
            本地文件
          </button>
          <button
            className={tab === 'text' ? 'active' : ''}
            onClick={() => {
              setTab('text');
              setError('');
            }}
          >
            <FileCode2 size={16} />
            粘贴 SKILL.md
          </button>
        </div>
        {tab === 'folder' ? (
          <>
            <input
              hidden
              type="file"
              multiple
              ref={(node) => {
                folderInput.current = node;
                node?.setAttribute('webkitdirectory', '');
              }}
              onChange={(event) => void readFiles(event.target.files)}
              aria-label="选择 Skill 文件夹"
            />
            <input
              hidden
              type="file"
              accept=".md"
              ref={fileInput}
              onChange={(event) => void readFiles(event.target.files)}
              aria-label="选择 SKILL.md 文件"
            />
            <button
              className="import-dropzone"
              onClick={() => folderInput.current?.click()}
              disabled={busy}
            >
              <span className="dropzone-icon">
                <FolderOpen size={28} strokeWidth={1.5} />
              </span>
              <strong>{busy ? '正在读取…' : '选择 Skill 文件夹'}</strong>
              <span>选择包含 SKILL.md 的单个 Skill 目录</span>
              <small>仅在本机解析，不会上传文件</small>
            </button>
            <div className="file-alternative">
              也可以{' '}
              <button
                className="text-button"
                onClick={() => fileInput.current?.click()}
                disabled={busy}
              >
                选择 SKILL.md 文件 <ArrowRight size={13} />
              </button>
            </div>
            {selected && (
              <div className="import-preview">
                <FileCode2 size={21} />
                <div>
                  <strong>{selected.skill.name}</strong>
                  <small>
                    {selected.label} · {selected.skill.fileCount} 个文件
                  </small>
                  <p>{selected.skill.description}</p>
                </div>
              </div>
            )}
          </>
        ) : (
          <div>
            <label className="field-label">
              SKILL.md 内容
              <textarea
                className="manifest-input mono"
                aria-label="SKILL.md 内容"
                value={text}
                onChange={(event) => setText(event.target.value)}
                placeholder={placeholder}
                spellCheck={false}
              />
            </label>
            <button
              className="text-button template-button"
              onClick={() => setText(placeholder)}
              type="button"
            >
              填入示例模板
            </button>
          </div>
        )}
        {error && (
          <div className="form-error" role="alert">
            <AlertCircle size={16} />
            {error}
          </div>
        )}
        <div className="import-note">
          <strong>支持标准 Skill 格式</strong>
          <p>
            文件需包含 YAML 格式的 <code>name</code> 和 <code>description</code>
            。此版本保存说明和元数据，用于体验安装流程。
          </p>
        </div>
      </div>
      <div className="modal-footer">
        <button className="button secondary" onClick={onClose} disabled={busy}>
          取消
        </button>
        <button
          className="button primary"
          onClick={submit}
          disabled={busy || (tab === 'folder' ? !selected : !text.trim())}
        >
          <Upload size={16} />
          导入并预览
        </button>
      </div>
    </Modal>
  );
}
