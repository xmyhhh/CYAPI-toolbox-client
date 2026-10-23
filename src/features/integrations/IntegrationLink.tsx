import { useState, type ReactNode } from 'react';
import { nativeAvailable, nativeCall } from '../../lib/api/native';
import { openExternal } from '../../lib/api/update';

/** Open desktop links in the system browser, preserving the complete guide URL. */
export function IntegrationLink({
  href,
  integrationId,
  children,
  className = 'n-button',
}: {
  href: string;
  integrationId?: string;
  children: ReactNode;
  className?: string;
}) {
  const [error, setError] = useState('');
  return (
    <>
      <a
        className={className}
        href={href}
        target="_blank"
        rel="noreferrer"
        onClick={(event) => {
          if (!nativeAvailable) return;
          event.preventDefault();
          setError('');
          void (
            integrationId
              ? nativeCall('integration-open-download', { integrationId })
              : openExternal(href)
          ).catch((reason) => setError(reason instanceof Error ? reason.message : String(reason)));
        }}
      >
        {children}
      </a>
      {error && (
        <span className="n-config-note" role="alert">
          {error}
        </span>
      )}
    </>
  );
}
