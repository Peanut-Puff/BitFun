import React, { useCallback, useEffect, useRef, useState } from 'react';
import { LockKeyhole, RefreshCw } from 'lucide-react';
import { Button, IconButton } from '@/component-library';
import {
  feedbackAPI,
  FeedbackApiError,
  type FeedbackMessage,
  type FeedbackRecordSummary,
} from '@/infrastructure/api';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { useFeedbackInboxStore } from './feedbackInboxStore';

interface FeedbackConversationViewProps {
  record: FeedbackRecordSummary;
}

export const FeedbackConversationView: React.FC<FeedbackConversationViewProps> = ({ record }) => {
  const { t, formatDate } = useI18n('common');
  const [messages, setMessages] = useState<FeedbackMessage[]>([]);
  const [nextCursor, setNextCursor] = useState<string>();
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<FeedbackApiError | null>(null);
  const [ackError, setAckError] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const topSentinelRef = useRef<HTMLDivElement>(null);
  const visibleAdminTimesRef = useRef(new Set<string>());
  const lastReadThroughRef = useRef<string | null>(null);
  const queuedReadThroughRef = useRef<string | null>(null);
  const ackInFlightRef = useRef(false);
  const mountedRef = useRef(true);
  const applyServerStatus = useFeedbackInboxStore(state => state.applyServerStatus);
  const markInaccessible = useFeedbackInboxStore(state => state.markInaccessible);
  const refreshInbox = useFeedbackInboxStore(state => state.refresh);

  const handleConversationError = useCallback((caught: unknown) => {
    const normalized = caught instanceof FeedbackApiError
      ? caught
      : new FeedbackApiError('SERVICE_UNAVAILABLE', 'Feedback service is unavailable', true);
    if (isConversationAccessError(normalized.code)) {
      markInaccessible(record.feedbackId);
      return;
    }
    setError(normalized);
  }, [markInaccessible, record.feedbackId]);

  const loadLatest = useCallback(async (manual: boolean) => {
    if (manual) setRefreshing(true);
    else setLoading(true);
    setError(null);
    try {
      const page = await feedbackAPI.openConversation({ feedbackId: record.feedbackId });
      if (!mountedRef.current) return;
      setMessages(current => manual ? mergeMessages(current, page.messages) : page.messages);
      setNextCursor(page.nextCursor);
      setHasMore(page.hasMore);
      setError(page.syncError ?? null);
      if (!manual) {
        requestAnimationFrame(() => {
          const container = scrollRef.current;
          if (container) container.scrollTop = container.scrollHeight;
        });
      }
    } catch (caught) {
      if (mountedRef.current) handleConversationError(caught);
    } finally {
      if (mountedRef.current) {
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [handleConversationError, record.feedbackId]);

  const loadEarlier = useCallback(async () => {
    const container = scrollRef.current;
    if (!container || !hasMore || !nextCursor || loadingEarlier || refreshing) return;
    const previousHeight = container.scrollHeight;
    const previousTop = container.scrollTop;
    setLoadingEarlier(true);
    setError(null);
    try {
      const page = await feedbackAPI.openConversation({
        feedbackId: record.feedbackId,
        cursor: nextCursor,
      });
      if (!mountedRef.current) return;
      setMessages(current => mergeMessages(page.messages, current));
      setNextCursor(page.nextCursor);
      setHasMore(page.hasMore);
      requestAnimationFrame(() => {
        const current = scrollRef.current;
        if (current) current.scrollTop = previousTop + current.scrollHeight - previousHeight;
      });
    } catch (caught) {
      if (mountedRef.current) handleConversationError(caught);
    } finally {
      if (mountedRef.current) setLoadingEarlier(false);
    }
  }, [handleConversationError, hasMore, loadingEarlier, nextCursor, record.feedbackId, refreshing]);

  const flushReadAcknowledgement = useCallback(async () => {
    if (ackInFlightRef.current || document.visibilityState !== 'visible') return;
    ackInFlightRef.current = true;
    try {
      while (queuedReadThroughRef.current) {
        const requested = queuedReadThroughRef.current;
        queuedReadThroughRef.current = null;
        try {
          const result = await feedbackAPI.acknowledgeFeedback(record.feedbackId, requested);
          if (!mountedRef.current) return;
          lastReadThroughRef.current = laterTimestamp(
            lastReadThroughRef.current,
            result.readThrough,
          );
          applyServerStatus(record.feedbackId, result.feedbackStatus);
          setAckError(false);
          await refreshInbox(true);
        } catch (caught) {
          if (!mountedRef.current) return;
          const normalized = caught instanceof FeedbackApiError ? caught : null;
          if (normalized && isConversationAccessError(normalized.code)) {
            markInaccessible(record.feedbackId);
          } else {
            setAckError(true);
          }
          queuedReadThroughRef.current = null;
          break;
        }
      }
    } finally {
      ackInFlightRef.current = false;
    }
  }, [applyServerStatus, markInaccessible, record.feedbackId, refreshInbox]);

  const queueVisibleAcknowledgement = useCallback(() => {
    if (document.visibilityState !== 'visible') return;
    const latestVisible = [...visibleAdminTimesRef.current].reduce<string | null>(
      (latest, value) => laterTimestamp(latest, value),
      null,
    );
    if (!latestVisible || !isLaterTimestamp(latestVisible, lastReadThroughRef.current)) return;
    queuedReadThroughRef.current = laterTimestamp(
      queuedReadThroughRef.current,
      latestVisible,
    );
    void flushReadAcknowledgement();
  }, [flushReadAcknowledgement]);

  useEffect(() => {
    mountedRef.current = true;
    setMessages([]);
    setNextCursor(undefined);
    setHasMore(false);
    setError(null);
    setAckError(false);
    visibleAdminTimesRef.current.clear();
    lastReadThroughRef.current = null;
    queuedReadThroughRef.current = null;
    void loadLatest(false);
    return () => {
      mountedRef.current = false;
    };
  }, [loadLatest, record.feedbackId]);

  useEffect(() => {
    const root = scrollRef.current;
    const sentinel = topSentinelRef.current;
    if (!root || !sentinel || !hasMore) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) void loadEarlier();
    }, { root, threshold: 0.1 });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [hasMore, loadEarlier, messages.length]);

  useEffect(() => {
    const root = scrollRef.current;
    if (!root) return;
    const elements = Array.from(
      root.querySelectorAll<HTMLElement>('[data-admin-created-at]'),
    );
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const createdAt = (entry.target as HTMLElement).dataset.adminCreatedAt;
        if (!createdAt) continue;
        if (entry.isIntersecting) visibleAdminTimesRef.current.add(createdAt);
        else visibleAdminTimesRef.current.delete(createdAt);
      }
      queueVisibleAcknowledgement();
    }, { root, threshold: 0.6 });
    elements.forEach(element => observer.observe(element));
    const onVisibilityChange = () => queueVisibleAcknowledgement();
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      observer.disconnect();
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
  }, [messages, queueVisibleAcknowledgement]);

  return (
    <div className="bitfun-feedback__conversation">
      <div className="bitfun-feedback__conversation-toolbar">
        <span>{t('feedback.conversation.messages')}</span>
        <IconButton
          type="button"
          variant="ghost"
          size="small"
          tooltip={t('feedback.conversation.refresh')}
          aria-label={t('feedback.conversation.refresh')}
          disabled={loading || refreshing || loadingEarlier}
          isLoading={refreshing}
          onClick={() => void loadLatest(true)}
        >
          <RefreshCw size={15} aria-hidden="true" />
        </IconButton>
      </div>
      <div ref={scrollRef} className="bitfun-feedback__messages" aria-live="polite">
        <div ref={topSentinelRef} className="bitfun-feedback__message-sentinel" aria-hidden="true" />
        {loading || loadingEarlier ? (
          <div className="bitfun-feedback__message-state" role="status">
            {loadingEarlier
              ? t('feedback.conversation.loadingEarlier')
              : t('feedback.conversation.loading')}
          </div>
        ) : null}
        {error ? (
          <div className="bitfun-feedback__message-error" role="alert">
            <span>{conversationErrorText(error.code, t)}</span>
            <Button type="button" variant="ghost" size="small" onClick={() => void loadLatest(true)}>
              {t('feedback.actions.retry')}
            </Button>
          </div>
        ) : null}
        {ackError ? (
          <div className="bitfun-feedback__message-notice" role="status">
            {t('feedback.conversation.ackFailed')}
          </div>
        ) : null}
        {!loading && messages.length === 0 && !error ? (
          <div className="bitfun-feedback__message-empty">
            {t('feedback.conversation.empty')}
          </div>
        ) : null}
        {messages.map(message => (
          <article
            key={message.messageId}
            className={`bitfun-feedback__message is-${message.sender}`}
            data-admin-created-at={message.sender === 'admin' ? message.createdAt : undefined}
          >
            <header>
              <strong>{message.sender === 'admin'
                ? t('feedback.conversation.admin')
                : t('feedback.conversation.you')}</strong>
              <time dateTime={message.createdAt}>
                {formatMessageDate(message.createdAt, formatDate)}
              </time>
            </header>
            <p>{message.content}</p>
          </article>
        ))}
      </div>
      {record.status === 'resolved' ? (
        <div className="bitfun-feedback__resolved-notice">
          <LockKeyhole size={14} aria-hidden="true" />
          {t('feedback.conversation.resolvedReadonly')}
        </div>
      ) : null}
    </div>
  );
};

function mergeMessages(first: FeedbackMessage[], second: FeedbackMessage[]): FeedbackMessage[] {
  const merged = new Map<string, FeedbackMessage>();
  [...first, ...second].forEach(message => merged.set(message.messageId, message));
  return [...merged.values()].sort((left, right) =>
    left.createdAt.localeCompare(right.createdAt)
      || left.messageId.localeCompare(right.messageId));
}

function laterTimestamp(left: string | null, right: string): string {
  if (!left) return right;
  return Date.parse(right) > Date.parse(left) ? right : left;
}

function isLaterTimestamp(value: string, current: string | null): boolean {
  return !current || Date.parse(value) > Date.parse(current);
}

function isConversationAccessError(code: string): boolean {
  return [
    'CAPABILITY_UNAVAILABLE',
    'CAPABILITY_INVALID',
    'CAPABILITY_EXPIRED',
    'CAPABILITY_REVOKED',
    'CAPABILITY_REQUIRED',
    'FEEDBACK_ACCESS_DENIED',
    'FEEDBACK_ACCESS_UNAVAILABLE',
    'FEEDBACK_ACCESS_EXPIRED',
    'FEEDBACK_NOT_FOUND',
  ].includes(code);
}

function conversationErrorText(code: string, t: (key: string) => string): string {
  if (code === 'CACHE_SAVE_FAILED' || code === 'CACHE_RESET_FAILED') {
    return t('feedback.conversation.cacheFailed');
  }
  return t('feedback.conversation.syncFailed');
}

function formatMessageDate(
  value: string,
  formatDate: (date: Date | number, options?: Intl.DateTimeFormatOptions) => string,
): string {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return value;
  return formatDate(timestamp, {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}
