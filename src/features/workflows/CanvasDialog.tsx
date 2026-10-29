import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, Save } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { canvasUrl, WORKFLOW_ORIGIN, workflowApi } from './api';

interface CanvasState {
  loaded: boolean | string;
  editable: boolean;
  saving: boolean;
  saved: number | null;
  message: string;
  error: boolean;
}

export function CanvasDialog({
  kind,
  id,
  preview,
  apiKey,
  title,
  onClose,
}: {
  kind: 'templates' | 'drafts' | 'tasks';
  id: string;
  preview: boolean;
  apiKey: string;
  title: string;
  onClose: () => void;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [url, setUrl] = useState('');
  const [error, setError] = useState('');
  const [state, setState] = useState<CanvasState>({
    loaded: false,
    editable: false,
    saving: false,
    saved: null,
    message: '正在载入…',
    error: false,
  });

  useEffect(() => {
    let disposed = false;
    void workflowApi
      .canvasSession(apiKey, kind, id)
      .then((session) => {
        if (!disposed) setUrl(canvasUrl(kind, id, session.token, preview));
      })
      .catch((cause) => {
        if (!disposed)
          setError(cause instanceof Error ? cause.message : '画布会话创建失败，请重新打开');
      });
    return () => {
      disposed = true;
    };
  }, [apiKey, kind, id, preview]);

  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow) return;
      if (event.origin !== WORKFLOW_ORIGIN) return;
      const data = event.data as Partial<CanvasState> & { type?: string };
      if (!data || data.type !== 'cyapi-canvas:state') return;
      setState({
        loaded: data.loaded ?? false,
        editable: data.editable === true,
        saving: data.saving === true,
        saved: data.saved ?? null,
        message: typeof data.message === 'string' ? data.message : '',
        error: data.error === true,
      });
    };
    window.addEventListener('message', receive);
    return () => window.removeEventListener('message', receive);
  }, []);

  const send = (type: string) => {
    frame.current?.contentWindow?.postMessage({ type }, WORKFLOW_ORIGIN);
  };

  const save = () => {
    if (state.saving || !state.editable) return;
    setState((current) => ({ ...current, saving: true, message: '保存中…' }));
    send('cyapi-canvas:save');
  };

  return (
    <Modal
      title={title}
      onClose={onClose}
      className="wfx-canvas-dialog"
      busy={state.saving}
    >
      <div className="wfx-canvas-bar">
        <span className={state.error ? 'error' : ''} role="status">
          {state.message}
        </span>
        <button
          type="button"
          className="wfx-button small primary"
          disabled={state.loaded !== true || !state.editable || state.saving}
          onClick={save}
        >
          {state.saving ? <LoaderCircle className="n-spin" size={14} /> : <Save size={14} />}
          保存画布
        </button>
      </div>
      <div className="wfx-canvas-frame">
        {error ? (
          <p className="wfx-canvas-error" role="alert">
            {error}
          </p>
        ) : url ? (
          <iframe
            ref={frame}
            title="ComfyUI 原生工作流画布"
            src={url}
            sandbox="allow-scripts allow-same-origin allow-downloads"
            onLoad={() => send('cyapi-canvas:request-state')}
          />
        ) : (
          <p className="wfx-canvas-error" role="status">
            <LoaderCircle className="n-spin" size={18} /> 正在创建画布会话…
          </p>
        )}
      </div>
    </Modal>
  );
}
