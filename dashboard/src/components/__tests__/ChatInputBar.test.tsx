import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentMetadata } from '../../types';
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

const baseProps = {
  onSend,
  onStop,
  agentId: 'agent.a',
  agentName: 'Sapphy',
  agents: [
    { id: 'agent.a', name: 'Sapphy' },
    { id: 'agent.b', name: 'KS22' },
  ] as AgentMetadata[],
  onSwitchAgent,
};

function draw(over: Partial<React.ComponentProps<typeof ChatInputBar>> = {}) {
  return render(
    <ChatInputBar
      onSend={onSend}
      onStop={onStop}
      agentId="agent.a"
      agentName="Sapphy"
      agents={
        [
          { id: 'agent.a', name: 'Sapphy' },
          { id: 'agent.b', name: 'KS22' },
        ] as AgentMetadata[]
      }
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

describe("'/' from anywhere", () => {
  it('puts the caret in the composer when nothing else is being typed in', () => {
    draw();
    expect(document.activeElement).not.toBe(box());
    const key = new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true });
    document.body.dispatchEvent(key);
    expect(document.activeElement).toBe(box());
    // The slash itself is not written into the composer.
    expect(key.defaultPrevented).toBe(true);
  });

  it('leaves the key alone when the composer is behind another screen', () => {
    render(
      <div className="hidden">
        <ChatInputBar onSend={onSend} onStop={onStop} agentId="agent.a" agentName="Sapphy" />
      </div>,
    );
    const key = new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true });
    document.body.dispatchEvent(key);
    expect(document.activeElement).not.toBe(box());
    expect(key.defaultPrevented).toBe(false);
  });

  it('does nothing while the composer cannot be written in', () => {
    draw({ disabled: true });
    const key = new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true });
    document.body.dispatchEvent(key);
    expect(document.activeElement).not.toBe(box());
    expect(key.defaultPrevented).toBe(false);
  });
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

  describe('with nobody to write to', () => {
    const drawInvitation = (onAccept = vi.fn()) => {
      draw({
        agentId: undefined,
        agentName: undefined,
        disabled: true,
        invitation: { label: 'Create an agent', placeholder: 'Name someone new to talk to', onAccept },
      });
      return onAccept;
    };
    const plus = () => screen.getByRole('button', { name: 'Create an agent' }) as HTMLButtonElement;

    it('says what the box is for, and can be typed into', () => {
      drawInvitation();
      expect(box().placeholder).toBe('Name someone new to talk to');
      expect(box().getAttribute('aria-label')).toBe('Name someone new to talk to');
      // Not disabled and not read-only: the name is typed here.
      expect(box().disabled).toBe(false);
      expect(box().readOnly).toBe(false);
      fireEvent.change(box(), { target: { value: 'Mira' } });
      expect(box().value).toBe('Mira');
    });

    it('does not open anything when the box is only pressed or typed into', () => {
      const onAccept = drawInvitation();
      fireEvent.click(box());
      fireEvent.keyDown(box(), { key: 'a' });
      fireEvent.keyDown(box(), { key: ' ' });
      fireEvent.keyDown(box(), { key: 'Tab' });
      expect(onAccept).not.toHaveBeenCalled();
    });

    it('hands over the typed name on Enter, trimmed, and keeps a name on one line', () => {
      const onAccept = drawInvitation();
      fireEvent.change(box(), { target: { value: '  Mira\nVale ' } });
      expect(box().value).toBe('  MiraVale ');
      fireEvent.keyDown(box(), { key: 'Enter' });
      expect(onAccept).toHaveBeenCalledTimes(1);
      expect(onAccept).toHaveBeenCalledWith('MiraVale');
      expect(onSend).not.toHaveBeenCalled();
    });

    it('does not take the Enter that ends IME composition for the end of the name', () => {
      const onAccept = drawInvitation();
      fireEvent.change(box(), { target: { value: 'みら' } });
      fireEvent.keyDown(box(), { key: 'Enter', isComposing: true });
      fireEvent.keyDown(box(), { key: 'Enter', keyCode: 229 });
      expect(onAccept).not.toHaveBeenCalled();
      fireEvent.keyDown(box(), { key: 'Enter' });
      expect(onAccept).toHaveBeenCalledWith('みら');
    });

    it('has a plus where the send arrow was: pressable with no name, and it sends nothing', () => {
      const onAccept = drawInvitation();
      expect(screen.queryByLabelText('chat_input.send')).toBeNull();
      expect(plus().disabled).toBe(false);
      expect(plus().className).toContain('nobody');
      fireEvent.click(plus());
      expect(onAccept).toHaveBeenCalledTimes(1);
      expect(onAccept).toHaveBeenCalledWith('');
      fireEvent.change(box(), { target: { value: 'Mira' } });
      fireEvent.click(plus());
      expect(onAccept).toHaveBeenLastCalledWith('Mira');
      expect(onAccept).toHaveBeenCalledTimes(2);
      expect(onSend).not.toHaveBeenCalled();
    });

    it('offers no attachment, and the hint keeps its place but says nothing', () => {
      drawInvitation();
      // The hint keeps its place (the box must not move as faces turn).
      expect(screen.getByText('chat_input.hint').style.visibility).toBe('hidden');
      expect(screen.queryByLabelText('chat_input.attach_image')).toBeNull();
    });

    it('keeps a half-written message out of the name, and the name out of the message', () => {
      const { rerender } = draw();
      fireEvent.change(box(), { target: { value: 'a message to Sapphy' } });
      const onAccept = vi.fn();
      rerender(
        <ChatInputBar
          {...baseProps}
          agentId={undefined}
          agentName={undefined}
          disabled
          invitation={{ label: 'Create an agent', placeholder: 'Name someone new to talk to', onAccept }}
        />,
      );
      expect(box().value).toBe('');
      fireEvent.change(box(), { target: { value: 'Mira' } });
      rerender(<ChatInputBar {...baseProps} />);
      expect(box().value).toBe('a message to Sapphy');
      // Turning back: the name typed before is not kept for the next somebody.
      rerender(
        <ChatInputBar
          {...baseProps}
          agentId={undefined}
          agentName={undefined}
          disabled
          invitation={{ label: 'Create an agent', placeholder: 'Name someone new to talk to', onAccept }}
        />,
      );
      expect(box().value).toBe('');
    });

    it('an ordinary composer is none of that', () => {
      draw();
      expect(box().readOnly).toBe(false);
      expect((screen.getByLabelText('chat_input.send') as HTMLButtonElement).className).not.toContain('nobody');
      expect(screen.getByText('chat_input.hint').style.visibility).toBe('');
      expect(screen.getByLabelText('chat_input.attach_image')).toBeTruthy();
    });
  });
});
