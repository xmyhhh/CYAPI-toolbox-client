import { useState } from 'react';
import { BookOpen, Check, Copy } from 'lucide-react';
import type { IntegrationDescriptor } from '../../lib/api/native';

export function IntegrationTutorial({ descriptor }: { descriptor: IntegrationDescriptor }) {
  const [copied, setCopied] = useState('');
  const [error, setError] = useState('');
  const guide = descriptor.guide;
  async function copy(prompt: string) {
    try {
      await navigator.clipboard.writeText(prompt);
      setCopied(prompt);
      setError('');
    } catch {
      setError('复制未完成，请选中下方示例文字手动复制。');
    }
  }
  return (
    <article className="n-panel n-integration-tutorial">
      <header>
        <BookOpen size={18} />
        <h3>接入教程</h3>
        <span>从准备到第一次使用</span>
      </header>
      {guide?.preparation.length ? (
        <section>
          <h4>先准备好这些</h4>
          <ul>
            {guide.preparation.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
      ) : null}
      <section>
        <h4>按步骤完成接入</h4>
        <ol className="n-setup-steps">
          {(
            guide?.steps ??
            descriptor.steps.map((description, i) => ({ title: `步骤 ${i + 1}`, description }))
          ).map((step, i) => (
            <li key={step.title}>
              <span>{String(i + 1).padStart(2, '0')}</span>
              <div>
                <h5>{step.title}</h5>
                <p>{step.description}</p>
              </div>
            </li>
          ))}
        </ol>
      </section>
      {guide?.examples.length ? (
        <section>
          <h4>在 AI 工具里试一试</h4>
          <p className="n-config-note">重新打开会话，复制示例请求；按任务替换素材、题名或目录。</p>
          {guide.examples.map((example) => (
            <div className="n-integration-example" key={example.title}>
              <div>
                <strong>{example.title}</strong>
                <button className="n-button" onClick={() => void copy(example.prompt)}>
                  {copied === example.prompt ? <Check size={13} /> : <Copy size={13} />}
                  {copied === example.prompt ? '已复制' : '复制请求'}
                </button>
              </div>
              <p>{example.prompt}</p>
              <small>
                <strong>预期结果：</strong>
                {example.expected}
              </small>
            </div>
          ))}
          <span role="status" className="n-config-note">
            {error || (copied ? '示例已复制，可粘贴到 AI 对话。' : '')}
          </span>
        </section>
      ) : (
        <p className="n-config-note">此目录尚未提供示例任务，请参考软件说明完成一次实际调用。</p>
      )}
      {guide?.limitations.length ? (
        <section>
          <h4>使用范围</h4>
          <ul>
            {guide.limitations.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
      ) : null}
      {guide?.troubleshooting.length ? (
        <section>
          <h4>遇到问题时</h4>
          {guide.troubleshooting.map((item) => (
            <details key={item.symptom}>
              <summary>{item.symptom}</summary>
              <p>{item.solution}</p>
            </details>
          ))}
        </section>
      ) : null}
    </article>
  );
}
