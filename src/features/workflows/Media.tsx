import { useEffect, useRef, useState } from 'react';
import { FileText } from 'lucide-react';
import { fetchMedia, fetchText } from './api';
import type { WorkflowOutput } from './types';

function useBlobUrl(path?: string, apiKey?: string) {
  const [url, setUrl] = useState('');
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!path) {
      setUrl('');
      return;
    }
    const controller = new AbortController();
    let objectUrl = '';
    setError(false);
    setUrl('');
    void fetchMedia(path, apiKey || undefined, controller.signal)
      .then((blob) => {
        objectUrl = URL.createObjectURL(blob);
        setUrl(objectUrl);
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      });
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [path, apiKey]);
  return { url, error };
}

export function AuthenticatedImage({
  path,
  apiKey,
  alt,
  className,
}: {
  path?: string;
  apiKey?: string;
  alt: string;
  className?: string;
}) {
  const { url, error } = useBlobUrl(path, apiKey);
  if (error)
    return (
      <span className={`wfx-media-fallback ${className || ''}`} role="img" aria-label={`${alt}（无法加载）`}>
        无法加载
      </span>
    );
  if (!url)
    return <span className={`wfx-media-placeholder ${className || ''}`} aria-hidden="true" />;
  return <img className={className} src={url} alt={alt} />;
}

export function AuthenticatedVideo({
  path,
  apiKey,
  label,
  poster,
}: {
  path?: string;
  apiKey?: string;
  label: string;
  poster?: string;
}) {
  const { url, error } = useBlobUrl(path, apiKey);
  if (error) return <p className="wfx-media-fallback">视频无法加载，请下载后查看。</p>;
  if (!url) return <span className="wfx-media-placeholder wfx-media-video" aria-hidden="true" />;
  return <video className="wfx-media-video" src={url} poster={poster} controls loop playsInline aria-label={label} />;
}

export function AuthenticatedAudio({ path, apiKey }: { path?: string; apiKey?: string }) {
  const { url, error } = useBlobUrl(path, apiKey);
  if (error) return <p className="wfx-media-fallback">音频无法加载。</p>;
  if (!url) return null;
  return <audio className="wfx-audio" src={url} controls preload="metadata" aria-label="语音输入" />;
}

interface Mesh {
  vertices: number[][];
  faces: number[][];
}

function parseObj(source: string): Mesh {
  const vertices: number[][] = [];
  const faces: number[][] = [];
  for (const line of source.split('\n')) {
    const fields = line.trim().split(/\s+/);
    if (fields[0] === 'v') vertices.push(fields.slice(1, 4).map(Number));
    if (fields[0] === 'f') faces.push(fields.slice(1).map((value) => Number(value.split('/')[0]) - 1));
  }
  if (
    !vertices.length ||
    vertices.length > 1000 ||
    vertices.some(
      (vertex) =>
        vertex.length !== 3 ||
        vertex.some((value) => !Number.isFinite(value) || Math.abs(value) > 1e4),
    ) ||
    faces.some((face) => !face.length || face.some((index) => !Number.isInteger(index) || !vertices[index]))
  )
    throw new Error('invalid mesh');
  return { vertices, faces };
}

export function MeshPreview({
  path,
  apiKey,
  label,
}: {
  path: string;
  apiKey?: string;
  label: string;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState(false);
  const mesh = useRef<Mesh | null>(null);
  const view = useRef({ yaw: 0.6, pitch: 0.35 });
  const drag = useRef<[number, number] | null>(null);

  const draw = () => {
    const node = canvas.current;
    const value = mesh.current;
    if (!node || !value) return;
    const context = node.getContext('2d');
    if (!context) return;
    const width = node.width;
    const height = node.height;
    const scale = Math.min(width, height) / 3.8;
    context.fillStyle = '#172738';
    context.fillRect(0, 0, width, height);
    const { yaw, pitch } = view.current;
    const points = value.vertices.map(([x, y, z]) => {
      const a = x * Math.cos(yaw) + z * Math.sin(yaw);
      const b = -x * Math.sin(yaw) + z * Math.cos(yaw);
      const c = y * Math.cos(pitch) - b * Math.sin(pitch);
      return [width / 2 + a * scale, height / 2 + (scale * height) / width - c * scale];
    });
    context.strokeStyle = '#78d9b1';
    context.lineWidth = 2;
    for (const face of value.faces) {
      context.beginPath();
      face.forEach((index, position) =>
        position ? context.lineTo(points[index][0], points[index][1]) : context.moveTo(points[index][0], points[index][1]),
      );
      context.closePath();
      context.stroke();
    }
    context.fillStyle = '#ffffff';
    context.font = '12px "Segoe UI", system-ui, sans-serif';
    context.fillText(`MOCK · ${value.vertices.length} vertices / ${value.faces.length} faces`, 14, 22);
  };

  useEffect(() => {
    const controller = new AbortController();
    setError(false);
    mesh.current = null;
    void fetchText(path, apiKey || undefined, controller.signal)
      .then((source) => {
        mesh.current = parseObj(source);
        draw();
      })
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      });
    return () => controller.abort();
    // `draw` only reads the refs updated above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, apiKey]);

  if (error) return <p className="wfx-media-fallback">3D 网格预览不可用，请下载 OBJ 查看。</p>;
  return (
    <div className="wfx-mesh">
      <canvas
        ref={canvas}
        width={520}
        height={320}
        aria-label={label}
        onPointerDown={(event) => {
          drag.current = [event.clientX, event.clientY];
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (!drag.current) return;
          view.current.yaw += (event.clientX - drag.current[0]) * 0.012;
          view.current.pitch += (event.clientY - drag.current[1]) * 0.012;
          drag.current = [event.clientX, event.clientY];
          draw();
        }}
        onPointerUp={() => {
          drag.current = null;
        }}
        onPointerCancel={() => {
          drag.current = null;
        }}
      />
      <p>{label} · 拖动旋转</p>
    </div>
  );
}

export function OutputMedia({ output, apiKey }: { output: WorkflowOutput; apiKey?: string }) {
  const kind = output.kind || 'image';
  if (kind === 'text')
    return (
      <div className="wfx-text-output">
        <p className="wfx-transcript">
          <FileText size={15} aria-hidden="true" />
          {output.preview || 'Mock 转写文本，请下载查看。'}
        </p>
      </div>
    );
  if (kind === 'video')
    return <AuthenticatedVideo path={output.url} apiKey={apiKey} label={output.title || 'Mock 视频'} />;
  if (kind === 'mesh')
    return <MeshPreview path={output.url} apiKey={apiKey} label={output.preview || '3D 网格'} />;
  return <AuthenticatedImage path={output.url} apiKey={apiKey} alt={output.title || 'Mock 图片结果'} className="wfx-artifact" />;
}
