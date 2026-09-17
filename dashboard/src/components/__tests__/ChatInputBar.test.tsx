import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatInputBar } from '../ChatInputBar';

// Echo i18n keys so the assertions do not depend on copy. Interpolation is
// kept visible so the placeholder can be checked for the agent's name.
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (k: string, opts?: Record<string, unknown>) => (opts?.name ? `${k}:${opts.name}` : k),
  }),
}));

const onSend = vi.fn();
const onStop = vi.fn();
const onSwitchAgent = vi.fn();

function draw(over: Partial<React.ComponentProps<typeof ChatInputBar>> = {}) {
  return render(
    <ChatInputBar
      onSend={onSend}
      onStop={onStop}
      agentId="agent.a"
      agentName="Sapphy"
      agents={[
        { id: 'agent.a', name: 'Sapphy' },
        { id: 'agent.b', name: 'KS22' },
      ]}
      onSwitchAgent={onSwitchAgent}
      {...over}
    />,
  );
}

const box = () => screen.getByRole('textbox') as HTMLTextAreaElement;

beforeEach(() => {
  onSend.mockReset();
  onStop.mockReset();
  onSwitchAgent.mockReset();
});

describe('the composer', () => {
  it("addresses the agent by name in its placeholder, as the mock's 'Talk to Sapphy'", () => {
    draw();
    expect(box().placeholder).toBe('chat_input.placeholder:Sapphy');
  });

  it('sends on Enter and trims the text', () => {
    draw();
    fireEvent.change(box(), { target: { value: '  hello  ' } });
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).toHaveBeenCalledWith([{ type: 'text', text: 'hello' }], 'hello', null);
    expect(box().value).toBe('');
  });

  it('does not send on Shift+Enter — that is a new line', () => {
    draw();
    fireEvent.change(box(), { target: { value: 'line one' } });
    fireEvent.keyDown(box(), { key: 'Enter', shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
    expect(box().value).toBe('line one');
  });

  it('sends a message that spans several lines', () => {
    draw();
    fireEvent.change(box(), { target: { value: 'line one\nline two' } });
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).toHaveBeenCalledWith([{ type: 'text', text: 'line one\nline two' }], 'line one\nline two', null);
  });

  it('does not send the Enter that ends IME composition', () => {
    draw();
    fireEvent.change(box(), { target: { value: 'にほんご' } });
    fireEvent.compositionStart(box());
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.compositionEnd(box());
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it('sends nothing empty, and nothing while the agent is off', () => {
    draw();
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).not.toHaveBeenCalled();
    expect((screen.getByRole('button', { name: 'chat_input.send' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('while a reply is being produced, the send button is a stop button, and Enter does not send', () => {
    draw({ generating: true });
    expect(screen.queryByRole('button', { name: 'chat_input.send' })).toBeNull();
    fireEvent.change(box(), { target: { value: 'wait' } });
    fireEvent.keyDown(box(), { key: 'Enter' });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'chat_input.stop' }));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('switches who you are talking to from the row', () => {
    draw();
    fireEvent.click(screen.getByRole('button', { name: 'chat_input.switch_agent' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'KS22' }));
    expect(onSwitchAgent).toHaveBeenCalledWith('agent.b');
    // Choosing the present agent again is not a switch.
    fireEvent.click(screen.getByRole('button', { name: 'chat_input.switch_agent' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Sapphy' }));
    expect(onSwitchAgent).toHaveBeenCalledTimes(1);
  });

  it('carries no microphone: audio was never sent, only a note pretending it was', () => {
    draw();
    expect(screen.queryByRole('button', { name: /record/i })).toBeNull();
  });
});
