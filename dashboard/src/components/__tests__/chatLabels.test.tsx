import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { BranchNavigator } from '../BranchNavigator';
import { CodeBlock } from '../CodeBlock';
import { MarkdownRenderer } from '../MarkdownRenderer';

// These labels used to be English literals that no language pack could reach.
vi.mock('react-i18next', () => {
  const t = (key: string, values?: Record<string, unknown>) => (values ? `${key} ${JSON.stringify(values)}` : key);
  return { useTranslation: () => ({ t, i18n: { language: 'en' } }) };
});

describe('conversation labels go through the language pack', () => {
  it('names the branch buttons from the pack', () => {
    render(<BranchNavigator count={2} activeIndex={0} indices={[0, 1]} onNavigate={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'console.branch_previous' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'console.branch_next' })).toBeEnabled();
  });

  it('titles the code block actions from the pack, including the copied state', async () => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn(async () => undefined) } });
    render(<CodeBlock code="print(1)" language="python" />);
    expect(screen.getByTitle('console.code_download')).toBeInTheDocument();
    fireEvent.click(screen.getByTitle('console.code_copy'));
    expect(await screen.findByTitle('console.code_copied')).toBeInTheDocument();
  });

  it('describes a long code block moved to the panel in a sentence from the pack', () => {
    const body = Array.from({ length: 16 }, (_, i) => `line ${i}`).join('\n');
    const { container } = render(<MarkdownRenderer content={`\`\`\`python\n${body}\n\`\`\``} onCodeBlock={vi.fn()} />);
    const placeholder = container.querySelector('.artifact-placeholder') as HTMLElement;
    expect(placeholder).not.toBeNull();
    expect(placeholder.textContent).toContain('console.code_summary');
    expect(placeholder.textContent).toContain('"lang":"python","lines":16');
    expect(placeholder.textContent).toContain('console.code_in_panel');
    expect(placeholder.textContent).not.toContain('View in panel');
  });
});
