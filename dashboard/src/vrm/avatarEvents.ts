import { API_BASE } from '../services/api';
import type { AvatarAgentState } from './engine/types';
import type { VisemeEntry } from './engine/VisemePlayer';
import type { VrmAnimationController } from './engine/VrmAnimationController';

export type AvatarEvent = { type: string; data: Record<string, unknown> };
/** The same event bridge is used by the room and the desktop viewer. */
export function applyAvatarEvent(
  event: AvatarEvent,
  agentId: string,
  controller: VrmAnimationController,
  onState: (state: AvatarAgentState) => void,
) {
  if (event.type === 'McpNotification') {
    const params = event.data.params as { channel?: string; data?: Record<string, unknown> } | undefined;
    const d = params?.data;
    if (!d || d.agent_id !== agentId) return;
    switch (params?.channel) {
      case 'avatar_set_expression':
        controller.setExpression(String(d.expression), Number(d.intensity ?? 1));
        break;
      case 'avatar_set_pose':
        void controller.setPose(String(d.pose), Number(d.transition ?? 0.5));
        break;
      case 'avatar_set_idle_behavior':
        controller.setIdleBehavior(d);
        break;
      case 'viseme_correction':
        controller.playVisemes((d.entries as VisemeEntry[]) ?? []);
        break;
      case 'avatar_speech_play': {
        const timeline = (d.viseme_timeline as VisemeEntry[]) ?? [];
        const offset = Number(d.audio_offset_ms ?? 0);
        if (typeof d.audio_data === 'string') void controller.playSpeechData(d.audio_data, timeline, offset);
        else if (typeof d.audio_url === 'string')
          void controller.playSpeech(`${API_BASE}${d.audio_url}`, timeline, offset);
        break;
      }
    }
    return;
  }
  if (event.data.agent_id !== agentId) return;
  switch (event.type) {
    case 'AgentThinking':
      onState('thinking');
      break;
    case 'ThoughtResponse':
      onState('responding');
      break;
    case 'AgenticLoopCompleted':
    case 'ResponseStopped':
      onState('idle');
      controller.stopVisemesSafe();
      break;
  }
}
