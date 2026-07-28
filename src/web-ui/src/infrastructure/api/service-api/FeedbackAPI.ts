import { flowChatStore } from '@/flow_chat/store/FlowChatStore';
import { api } from './ApiClient';

export const FEEDBACK_CONTENT_MAX_CHARS = 2_000;

export type FeedbackCategory = 'runtime_error' | 'feature_request' | 'usage_question' | 'other';
export type FeedbackStatus = 'submitted' | 'in_progress' | 'waiting_user' | 'resolved';

export interface SubmitFeedbackInput {
  category: FeedbackCategory;
  content: string;
  includeCorrelation: boolean;
}

export interface SubmitFeedbackResult {
  feedbackId: string;
  status: FeedbackStatus;
  inboxCursor: string;
}

interface FeedbackCommandErrorShape {
  code?: string;
  message?: string;
  retryable?: boolean;
  requestId?: string;
  retryAfterSeconds?: number;
}

export class FeedbackApiError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly retryable: boolean,
    public readonly requestId?: string,
    public readonly retryAfterSeconds?: number,
  ) {
    super(message);
    this.name = 'FeedbackApiError';
  }
}

export class FeedbackAPI {
  correlationAvailable(): boolean {
    return Boolean(flowChatStore.getState().activeSessionId);
  }

  async submitFeedback(input: SubmitFeedbackInput): Promise<SubmitFeedbackResult> {
    const submit = await this.prepareSubmission(input);
    return submit();
  }

  async prepareSubmission(
    input: SubmitFeedbackInput,
  ): Promise<() => Promise<SubmitFeedbackResult>> {
    const sessionIdHash = input.includeCorrelation
      ? await this.activeSessionIdHash()
      : undefined;
    const request = {
      category: input.category,
      content: input.content,
      sessionIdHash,
    };
    return () => this.invoke<SubmitFeedbackResult>('submit_feedback', request);
  }

  private async invoke<T>(command: string, request: Record<string, unknown>): Promise<T> {
    try {
      return await api.invoke<T>(command, { request }, { retries: 0 });
    } catch (error) {
      throw normalizeFeedbackError(error);
    }
  }

  private async activeSessionIdHash(): Promise<string | undefined> {
    const sessionId = flowChatStore.getState().activeSessionId;
    if (!sessionId || !globalThis.crypto?.subtle) return undefined;
    const bytes = new TextEncoder().encode(sessionId);
    const digest = await globalThis.crypto.subtle.digest('SHA-256', bytes);
    return Array.from(
      new Uint8Array(digest),
      value => value.toString(16).padStart(2, '0'),
    ).join('');
  }
}

export function truncateFeedbackContent(value: string): string {
  const characters = Array.from(value);
  return characters.length <= FEEDBACK_CONTENT_MAX_CHARS
    ? value
    : characters.slice(0, FEEDBACK_CONTENT_MAX_CHARS).join('');
}

export function feedbackContentLength(value: string): number {
  return Array.from(value).length;
}

export function normalizeFeedbackError(error: unknown): FeedbackApiError {
  if (error instanceof FeedbackApiError) return error;
  const value = error as FeedbackCommandErrorShape | null;
  if (value && typeof value === 'object' && typeof value.code === 'string') {
    return fromShape(value);
  }
  const message = error instanceof Error ? error.message : String(error);
  const structured = parseStructuredError(message);
  if (structured?.code) return fromShape(structured);
  return new FeedbackApiError('SERVICE_UNAVAILABLE', 'Feedback service is unavailable', true);
}

function fromShape(value: FeedbackCommandErrorShape): FeedbackApiError {
  return new FeedbackApiError(
    value.code ?? 'UNKNOWN_ERROR',
    value.message ?? 'Feedback request could not be completed',
    value.retryable ?? false,
    value.requestId,
    value.retryAfterSeconds,
  );
}

function parseStructuredError(message: string): FeedbackCommandErrorShape | null {
  const start = message.indexOf('{');
  if (start < 0) return null;
  try {
    return JSON.parse(message.slice(start)) as FeedbackCommandErrorShape;
  } catch {
    return null;
  }
}

export const feedbackAPI = new FeedbackAPI();
