import { Download, Info } from 'lucide-react';
import { nativeAvailable } from '../../lib/api/native';
import type { IntegrationDescriptor, IntegrationEntry } from '../../lib/api/native';
import { IntegrationLink } from './IntegrationLink';

export function VersionPanel({
  descriptor,
  entry,
}: {
  descriptor: IntegrationDescriptor;
  entry?: IntegrationEntry;
}) {
  const compatibility = descriptor.compatibility;
  const detected = entry?.software;
  return (
    <section
      className="n-panel n-software-version"
      aria-label="软件版本与官方下载"
      data-state={detected?.state}
    >
      <div className="n-software-version-top">
        <h3>软件版本与兼容性</h3>
        <IntegrationLink href={descriptor.downloadUrl} integrationId={descriptor.id}>
          <Download size={14} />
          官方下载 / 更新
        </IntegrationLink>
      </div>
      <dl>
        <div>
          <dt>支持范围</dt>
          <dd>{compatibility?.supported ?? `${descriptor.name} ${descriptor.minimumVersion}`}</dd>
        </div>
        <div>
          <dt>本机版本</dt>
          <dd title={detected?.source ?? undefined}>
            {detected?.version ?? (nativeAvailable ? '版本未确认' : '桌面版可检测')}
          </dd>
        </div>
        <div>
          <dt>已实测版本</dt>
          <dd>
            {compatibility?.testedVersions.length
              ? compatibility.testedVersions.join('、')
              : '暂无软件内实测版本记录'}
          </dd>
        </div>
      </dl>
      <p className="n-version-message" role="status">
        <Info size={15} />
        <span>
          {detected?.found === false
            ? '未在常见安装位置发现软件；便携版或自定义目录请手动核对版本。'
            : (detected?.message ?? '请按支持范围核对软件版本，再完成教程中的首次任务验证。')}
        </span>
      </p>
      {compatibility?.notes.length ? (
        <ul>
          {compatibility.notes.map((note) => (
            <li key={note}>{note}</li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
