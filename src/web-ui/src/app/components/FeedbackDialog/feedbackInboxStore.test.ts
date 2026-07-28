import { beforeEach, describe, expect, it, vi } from 'vitest';

const getAccessState = vi.fn();
const listFeedbackRecords = vi.fn();

vi.mock('@/infrastructure/api', () => ({
  feedbackAPI: { getAccessState, listFeedbackRecords },
  normalizeFeedbackError: (error: unknown) => error,
}));

describe('feedbackInboxStore', () => {
  beforeEach(async () => {
    vi.resetModules();
    getAccessState.mockReset();
    listFeedbackRecords.mockReset();
  });

  it('does not inspect or query access in privacy-not-accepted mode', async () => {
    const { useFeedbackInboxStore } = await import('./feedbackInboxStore');
    await useFeedbackInboxStore.getState().initializeForMode('privacy_not_accepted');
    expect(getAccessState).not.toHaveBeenCalled();
    expect(listFeedbackRecords).not.toHaveBeenCalled();
  });

  it('checks once but does not enroll or query when there is no history', async () => {
    getAccessState.mockResolvedValue({
      hasHistory: false,
      canReuseAccess: false,
      cachedInbox: { items: [], hasMore: false },
    });
    const { useFeedbackInboxStore } = await import('./feedbackInboxStore');
    await useFeedbackInboxStore.getState().initializeForMode('full');
    await useFeedbackInboxStore.getState().initializeForMode('full');
    expect(getAccessState).toHaveBeenCalledTimes(1);
    expect(listFeedbackRecords).not.toHaveBeenCalled();
  });

  it('preserves cached records when an active refresh fails', async () => {
    const cached = {
      feedbackId: 'feedback-1',
      category: 'other',
      status: 'waiting_user',
      hasNewReply: true,
      createdAt: '2026-07-28T01:00:00Z',
      updatedAt: '2026-07-28T02:00:00Z',
      canOpen: true,
    };
    getAccessState.mockResolvedValue({
      hasHistory: true,
      canReuseAccess: true,
      cachedInbox: { items: [cached], nextCursor: 'cached-cursor', hasMore: true },
    });
    listFeedbackRecords.mockRejectedValue({ code: 'NETWORK_ERROR' });
    const { useFeedbackInboxStore } = await import('./feedbackInboxStore');

    expect(await useFeedbackInboxStore.getState().refresh(true)).toBe(false);
    expect(useFeedbackInboxStore.getState().records).toEqual([cached]);
    expect(useFeedbackInboxStore.getState().nextCursor).toBe('cached-cursor');
  });

  it('performs one startup Inbox query when full mode has reusable history', async () => {
    getAccessState.mockResolvedValue({
      hasHistory: true,
      canReuseAccess: true,
      cachedInbox: { items: [], hasMore: false },
    });
    listFeedbackRecords.mockResolvedValue({ items: [], hasMore: false });
    const { useFeedbackInboxStore } = await import('./feedbackInboxStore');

    await useFeedbackInboxStore.getState().initializeForMode('full');
    await useFeedbackInboxStore.getState().initializeForMode('full');

    expect(listFeedbackRecords).toHaveBeenCalledTimes(1);
    expect(listFeedbackRecords).toHaveBeenCalledWith({}, { userInitiated: false });
  });
});
