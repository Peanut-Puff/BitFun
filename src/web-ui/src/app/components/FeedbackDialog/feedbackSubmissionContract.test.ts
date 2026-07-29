import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const readSource = (relativePath: string): string =>
  readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8').replace(/\r\n?/g, '\n');

describe('OpenHarmony feedback submission contract', () => {
  it('keeps other platforms on the external GitCode route', () => {
    const footer = readSource('../NavPanel/components/PersistentFooterActions.tsx');

    expect(footer).toContain("systemInfo.platform === 'openharmony'");
    expect(footer).toContain('setShowFeedback(true)');
    expect(footer).toContain("systemAPI.openExternal('https://gitcode.com/OpenHarmonyPCDeveloper/BitFun/issues')");
  });

  it('requires total consent before a feedback request in not-accepted mode', () => {
    const dialog = readSource('./FeedbackDialog.tsx');
    const preparePosition = dialog.indexOf('await feedbackAPI.prepareSubmission({');
    const acceptPosition = dialog.indexOf('await accept({');
    const submitPosition = dialog.indexOf('await submitPreparedFeedback();');

    expect(preparePosition).toBeGreaterThan(0);
    expect(acceptPosition).toBeGreaterThan(preparePosition);
    expect(acceptPosition).toBeGreaterThan(0);
    expect(submitPosition).toBeGreaterThan(acceptPosition);
    expect(dialog).toContain("setSubmitError('PRIVACY_SAVE_FAILED')");
    expect(dialog).toContain('return;');
  });

  it('counts the privacy checkbox as draft state and freezes close while submitting', () => {
    const dialog = readSource('./FeedbackDialog.tsx');
    const layout = readSource('../../layout/AppLayout.tsx');

    expect(dialog).toContain('category || content || includeCorrelation || privacyChecked');
    expect(dialog).toContain('if (submitting || replyState.sending) return;');
    expect(dialog).toContain('showCloseButton={!submitting && !replyState.sending}');
    expect(dialog).toContain('closeOnOverlayClick={!submitting && !replyState.sending}');
    expect(dialog).toContain('registerCriticalOperationExitGuard');
    expect(layout).toContain('await confirmCriticalOperationExit()');
  });

  it('shows a single completion action after capability-backed success', () => {
    const dialog = readSource('./FeedbackDialog.tsx');
    const completeView = dialog.slice(
      dialog.indexOf('className="bitfun-feedback__complete"'),
      dialog.indexOf(') : (\n          <div ref={containerRef}'),
    );

    expect(completeView).toContain("t('shared:statuses.done')");
    expect(completeView).not.toContain('openGitCode');
    expect(completeView).not.toContain('feedback-submit');
  });

  it('uses the feedback container width for the 840px layout threshold', () => {
    const dialog = readSource('./FeedbackDialog.tsx');
    const styles = readSource('./FeedbackDialog.scss');

    expect(dialog).toContain('new ResizeObserver');
    expect(dialog).toContain('setWideLayout(width >= 840)');
    expect(styles).toContain('&.is-wide');
    expect(styles).toContain('grid-template-columns: minmax(300px, 36%) minmax(0, 1fr)');
  });

  it('keeps Mock request logs limited to a fixed stage and request id', () => {
    const mock = readSource('../../../../../../scripts/feedback-mock-server.mjs');

    expect(mock).toContain('logRequestStage(requestStage(request.method, url.pathname), requestId);');
    expect(mock).toContain("return method === 'GET' ? 'message_history' : method === 'POST' ? 'reply' : 'unknown';");
    expect(mock).toContain('process.stdout.write(`${JSON.stringify({ stage, requestId })}\\n`);');
    expect(mock).not.toContain('JSON.stringify({ stage, requestId, url');
    expect(mock).not.toContain('JSON.stringify({ stage, requestId, body');
  });
});
