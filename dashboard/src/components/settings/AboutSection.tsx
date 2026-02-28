import { SectionCard } from './common';

export function AboutSection() {
  return (
    <>
      <SectionCard title="ClotoCore">
        <div className="space-y-3">
          <p className="text-xs text-content-secondary leading-relaxed">
            AI agent orchestration platform built on a Rust kernel with MCP-based plugin architecture.
          </p>
          <div className="text-2xl font-mono font-black text-brand">v{__APP_VERSION__}</div>
        </div>
      </SectionCard>

      <SectionCard title="License">
        <div className="space-y-2">
          <p className="text-xs text-content-secondary">Business Source License 1.1</p>
          <p className="text-[10px] text-content-muted">Converts to MIT License on 2028-02-14</p>
        </div>
      </SectionCard>

      <SectionCard title="Links">
        <div className="space-y-3">
          {[
            { label: 'Repository', value: 'github.com/Cloto-dev/ClotoCore', href: 'https://github.com/Cloto-dev/ClotoCore' },
            { label: 'Contact', value: 'ClotoCore@proton.me', href: 'mailto:ClotoCore@proton.me' },
          ].map(link => (
            <div key={link.label} className="flex items-center justify-between">
              <span className="text-[10px] text-content-tertiary uppercase tracking-widest font-bold">{link.label}</span>
              <a
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-brand hover:underline font-mono"
              >
                {link.value}
              </a>
            </div>
          ))}
        </div>
      </SectionCard>
    </>
  );
}
