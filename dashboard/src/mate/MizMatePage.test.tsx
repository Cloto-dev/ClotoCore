import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeEach, expect, it, vi } from 'vitest';
import { MizMatePage } from './MizMatePage';

const m = vi.hoisted(() => ({
  agents: [{ id: 'agent.mio', name: 'Mio', metadata: {} }],
  selectedAgentId: 'agent.mio',
  setSelectedAgentId: vi.fn(),
  setSystemActive: vi.fn(),
  systemActive: false,
  processingAgentIds: new Set(),
  refetchAgents: vi.fn(),
  connection: { connected: true, checking: false },
  questions: 0,
}));
vi.mock('../contexts/ConnectionContext', () => ({ useConnection: () => m.connection }));
vi.mock('./usePartnerPresence', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./usePartnerPresence')>()),
  usePartnerQuestions: () => m.questions,
}));
beforeEach(() => {
  m.connection = { connected: true, checking: false };
  m.questions = 0;
  m.processingAgentIds = new Set();
});
vi.mock('../contexts/AgentContext', () => ({ useAgentContext: () => m }));
vi.mock('../contexts/ConversationContext', () => ({
  useConversations: () => ({ openFor: () => 'conversation', mountKeyFor: () => 'conversation' }),
}));
vi.mock('../hooks/useApi', () => ({ useApi: () => ({ apiKey: '' }) }));
vi.mock('../hooks/useUnreadAgents', () => ({ useReadOnOpen: vi.fn() }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => (values ? `${key} ${JSON.stringify(values)}` : key),
  }),
}));
vi.mock('../lib/agentIdentity', () => ({ AgentIcon: () => null }));
vi.mock('./SoftGlass', () => ({ attachSoftGlass: () => undefined }));
vi.mock('./PartnerStage', () => ({ PartnerStage: () => null }));
vi.mock('../components/AgentConsole', () => ({ AgentConsole: () => <input aria-label="conversation draft" /> }));
vi.mock('./PartnerMemories', () => ({
  PartnerMemories: ({ onTalk, onBack }: { onTalk: () => void; onBack: () => void }) => (
    <section aria-label="memories panel">
      <button type="button" onClick={onTalk}>
        talk from memories
      </button>
      <button type="button" onClick={onBack}>
        back from memories
      </button>
    </section>
  ),
}));
function Where() {
  return <output aria-label="path">{useLocation().pathname}</output>;
}
it('keeps the conversation draft mounted across expansion, closing and reopening', () => {
  render(
    <MemoryRouter>
      <MizMatePage />
    </MemoryRouter>,
  );
  expect(screen.queryByLabelText('conversation draft')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'mate.talk' }));
  fireEvent.change(screen.getByLabelText('conversation draft'), { target: { value: 'unfinished message' } });
  fireEvent.click(screen.getByRole('button', { name: 'mate.expand' }));
  expect(screen.getByLabelText('conversation draft')).toHaveValue('unfinished message');
  fireEvent.click(screen.getByRole('button', { name: 'mate.close_chat' }));
  fireEvent.click(screen.getByRole('button', { name: 'mate.talk' }));
  expect(screen.getByLabelText('conversation draft')).toHaveValue('unfinished message');
  expect(screen.getByRole('button', { name: 'mate.restore' })).toBeInTheDocument();
});

it('opens memories inside the companion home instead of leaving for the old screen', () => {
  render(
    <MemoryRouter>
      <MizMatePage />
      <Where />
    </MemoryRouter>,
  );
  fireEvent.click(screen.getByRole('button', { name: 'mate.talk' }));
  fireEvent.change(screen.getByLabelText('conversation draft'), { target: { value: 'kept draft' } });
  const rail = screen.getByRole('button', { name: 'mate.memories' });
  fireEvent.click(rail);
  expect(screen.getByLabelText('path')).toHaveTextContent(/^\/$/);
  expect(rail).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByLabelText('memories panel')).toBeInTheDocument();
  expect(screen.getByLabelText('conversation draft')).not.toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'talk from memories' }));
  expect(screen.queryByLabelText('memories panel')).not.toBeInTheDocument();
  expect(screen.getByLabelText('conversation draft')).toBeVisible();
  expect(screen.getByLabelText('conversation draft')).toHaveValue('kept draft');
  fireEvent.click(rail);
  fireEvent.click(screen.getByRole('button', { name: 'back from memories' }));
  expect(screen.queryByLabelText('memories panel')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'mate.room' })).toHaveAttribute('aria-pressed', 'true');
});

function room() {
  return render(
    <MemoryRouter>
      <MizMatePage />
    </MemoryRouter>,
  );
}

it('says the connection is lost and does not open a conversation that cannot load', () => {
  m.connection = { connected: false, checking: false };
  room();
  expect(screen.getByRole('status')).toHaveTextContent('mate.state_offline');
  const talk = screen.getByRole('button', { name: 'mate.talk' });
  expect(talk).toBeDisabled();
  fireEvent.click(talk);
  expect(screen.queryByLabelText('conversation draft')).not.toBeInTheDocument();
});

it('does not call the first unanswered health check a lost connection', () => {
  m.connection = { connected: false, checking: true };
  room();
  expect(screen.getByRole('status')).toHaveTextContent('mate.waiting');
  expect(screen.getByRole('button', { name: 'mate.talk' })).toBeEnabled();
});

it('tells the room a question is waiting and opens the conversation that holds it', () => {
  m.questions = 2;
  room();
  expect(screen.getByRole('status')).toHaveTextContent('mate.state_asking {"n":2}');
  fireEvent.click(screen.getByRole('button', { name: 'mate.review_question' }));
  expect(screen.getByLabelText('conversation draft')).toBeVisible();
});

it('shows thinking only for the partner in the room', () => {
  m.processingAgentIds = new Set(['agent.other']);
  const { unmount } = room();
  expect(screen.getByRole('status')).toHaveTextContent('mate.waiting');
  unmount();
  m.processingAgentIds = new Set(['agent.mio']);
  room();
  expect(screen.getByRole('status')).toHaveTextContent('mate.thinking');
});
