import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const readSource = (relativePath: string): string =>
  readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8').replace(/\r\n?/g, '\n');

describe('feedback conversation contract', () => {
  it('loads earlier cached messages from a top observer and preserves the scroll anchor', () => {
    const source = readSource('./FeedbackConversationView.tsx');

    expect(source).toContain('new IntersectionObserver');
    expect(source).toContain('topSentinelRef');
    expect(source).toContain('void loadEarlier()');
    expect(source).toContain('previousTop + current.scrollHeight - previousHeight');
    expect(source).not.toContain('feedback.conversation.loadMore');
  });

  it('acks only visible admin messages while the document is foreground-visible', () => {
    const source = readSource('./FeedbackConversationView.tsx');

    expect(source).toContain("document.visibilityState !== 'visible'");
    expect(source).toContain("'[data-admin-created-at]'");
    expect(source).toContain("message.sender === 'admin'");
    expect(source).toContain('feedbackAPI.acknowledgeFeedback(record.feedbackId, requested)');
    expect(source).toContain('result.readThrough');
    expect(source).toContain('result.feedbackStatus');
  });

  it('keeps refresh accessible and renders server content as plain React text', () => {
    const source = readSource('./FeedbackConversationView.tsx');

    expect(source).toContain('<IconButton');
    expect(source).toContain("tooltip={t('feedback.conversation.refresh')}");
    expect(source).toContain("aria-label={t('feedback.conversation.refresh')}");
    expect(source).toContain('<p>{message.content}</p>');
    expect(source).not.toContain('dangerouslySetInnerHTML');
  });
});
