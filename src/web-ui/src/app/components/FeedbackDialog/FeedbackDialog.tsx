import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { CheckCircle2, ExternalLink, Send } from 'lucide-react';
import {
  Button,
  Checkbox,
  confirmDialog,
  ConfirmDialog,
  Modal,
  Select,
  Switch,
  Textarea,
} from '@/component-library';
import { PrivacyStatementDialog } from '@/app/components/Privacy/PrivacyStatementDialog';
import { usePrivacy } from '@/app/components/Privacy/PrivacyContext';
import {
  feedbackAPI,
  FeedbackApiError,
  feedbackContentLength,
  systemAPI,
  truncateFeedbackContent,
  type FeedbackCategory,
} from '@/infrastructure/api';
import { useI18n } from '@/infrastructure/i18n/hooks/useI18n';
import { createLogger } from '@/shared/utils/logger';
import { registerCriticalOperationExitGuard } from '@/shared/services/criticalOperationExitGuard';
import './FeedbackDialog.scss';

const log = createLogger('FeedbackDialog');
const GITCODE_ISSUES_URL = 'https://gitcode.com/OpenHarmonyPCDeveloper/BitFun/issues';

interface FeedbackDialogProps {
  isOpen: boolean;
  onClose: () => void;
}

type SubmissionError = FeedbackApiError | 'PRIVACY_SAVE_FAILED' | null;

export const FeedbackDialog: React.FC<FeedbackDialogProps> = ({ isOpen, onClose }) => {
  const { t } = useI18n('common');
  const { status, accept } = usePrivacy();
  const [category, setCategory] = useState<FeedbackCategory | ''>('');
  const [content, setContent] = useState('');
  const [includeCorrelation, setIncludeCorrelation] = useState(false);
  const [privacyChecked, setPrivacyChecked] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<SubmissionError>(null);
  const [gitCodeError, setGitCodeError] = useState(false);
  const [showDiscardConfirm, setShowDiscardConfirm] = useState(false);
  const [showPrivacy, setShowPrivacy] = useState(false);
  const [wasTruncated, setWasTruncated] = useState(false);
  const [completed, setCompleted] = useState(false);
  const [retryWaitSeconds, setRetryWaitSeconds] = useState(0);

  const contentLength = feedbackContentLength(content);
  const correlationAvailable = feedbackAPI.correlationAvailable();
  const hasDraft = Boolean(
    category || content || includeCorrelation || privacyChecked,
  );
  const canSubmit = Boolean(
    category
      && content.trim()
      && privacyChecked
      && status?.policy
      && retryWaitSeconds === 0,
  );
  const categoryOptions = useMemo(() => [
    { value: 'runtime_error', label: t('feedback.categories.runtimeError') },
    { value: 'feature_request', label: t('feedback.categories.featureRequest') },
    { value: 'usage_question', label: t('feedback.categories.usageQuestion') },
    { value: 'other', label: t('feedback.categories.other') },
  ], [t]);

  useEffect(() => {
    if (!submitting) return;
    return registerCriticalOperationExitGuard(() => confirmDialog({
      title: t('feedback.exit.title'),
      message: t('feedback.exit.message'),
      confirmText: t('feedback.exit.quit'),
      cancelText: t('feedback.exit.wait'),
      confirmDanger: true,
      showCancel: true,
    }));
  }, [submitting, t]);

  useEffect(() => {
    if (retryWaitSeconds <= 0) return;
    const timer = window.setInterval(() => {
      setRetryWaitSeconds(current => Math.max(0, current - 1));
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [retryWaitSeconds]);

  const reset = useCallback(() => {
    setCategory('');
    setContent('');
    setIncludeCorrelation(false);
    setPrivacyChecked(false);
    setSubmitting(false);
    setSubmitError(null);
    setGitCodeError(false);
    setShowDiscardConfirm(false);
    setShowPrivacy(false);
    setWasTruncated(false);
    setCompleted(false);
    setRetryWaitSeconds(0);
  }, []);

  const closeImmediately = useCallback(() => {
    reset();
    onClose();
  }, [onClose, reset]);

  const requestClose = useCallback(() => {
    if (submitting) return;
    if (hasDraft && !completed) {
      setShowDiscardConfirm(true);
      return;
    }
    closeImmediately();
  }, [closeImmediately, completed, hasDraft, submitting]);

  const handleContentChange = (value: string) => {
    const truncated = truncateFeedbackContent(value);
    setWasTruncated(truncated !== value);
    setContent(truncated);
    setSubmitError(null);
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSubmit || !category || submitting || !status?.policy) return;
    setSubmitting(true);
    setSubmitError(null);
    try {
      const submitPreparedFeedback = await feedbackAPI.prepareSubmission({
        category,
        content: content.trim(),
        includeCorrelation,
      });
      if (status.effectiveMode !== 'full') {
        try {
          await accept({
            policyVersion: status.policy.policyVersion,
            consentVersion: status.policy.consentVersion,
            documentSha256: status.policy.documentSha256,
            locale: status.policy.locale,
          });
        } catch (error) {
          log.warn('Privacy consent could not be saved before feedback submission', error);
          setSubmitError('PRIVACY_SAVE_FAILED');
          return;
        }
      }
      await submitPreparedFeedback();
      setCompleted(true);
    } catch (error) {
      const feedbackError = asFeedbackError(error);
      setSubmitError(feedbackError);
      if (feedbackError.retryAfterSeconds) {
        setRetryWaitSeconds(Math.ceil(feedbackError.retryAfterSeconds));
      }
    } finally {
      setSubmitting(false);
    }
  };

  const openGitCode = () => {
    setGitCodeError(false);
    void systemAPI.openExternal(GITCODE_ISSUES_URL).catch(error => {
      log.warn('GitCode feedback page could not be opened', error);
      setGitCodeError(true);
    });
  };

  const displayError = (error: SubmissionError): string | null => {
    if (!error) return null;
    if (error === 'PRIVACY_SAVE_FAILED') return t('feedback.errors.privacySave');
    if (error.code === 'RATE_LIMITED' || error.code === 'FEEDBACK_QUOTA_EXCEEDED') {
      return t('feedback.errors.rateLimited', {
        seconds: retryWaitSeconds || error.retryAfterSeconds || 0,
      });
    }
    if (error.code === 'CAPABILITY_SAVE_FAILED') {
      return t('feedback.errors.capabilitySave');
    }
    if (error.code === 'FEEDBACK_NOT_CONFIGURED') return t('feedback.errors.notConfigured');
    if (error.code === 'REQUEST_TIMEOUT') return t('feedback.errors.timeout');
    if (error.code === 'NETWORK_ERROR' || error.code === 'SERVICE_UNAVAILABLE') {
      return t('feedback.errors.network');
    }
    return t('feedback.errors.generic', { code: error.code });
  };

  return (
    <>
      <Modal
        isOpen={isOpen}
        onClose={requestClose}
        title={t('header.feedback')}
        size="large"
        contentClassName="bitfun-feedback__modal-content"
        showCloseButton={!submitting}
        closeOnOverlayClick={!submitting}
        testId="feedback-dialog"
      >
        {completed ? (
          <div className="bitfun-feedback__complete" role="status">
            <CheckCircle2 size={34} aria-hidden="true" />
            <strong>{t('feedback.complete.title')}</strong>
            <span>{t('feedback.complete.description')}</span>
            <Button onClick={closeImmediately}>{t('shared:statuses.done')}</Button>
          </div>
        ) : (
          <form className="bitfun-feedback__form" onSubmit={handleSubmit}>
            <div className="bitfun-feedback__field">
              <label>{t('feedback.category')}<span aria-hidden="true">*</span></label>
              <Select
                value={category}
                placeholder={t('feedback.categoryPlaceholder')}
                options={categoryOptions}
                disabled={submitting}
                onChange={value => {
                  setCategory(value as FeedbackCategory);
                  setSubmitError(null);
                }}
                autoClose
                triggerTestId="feedback-category"
              />
            </div>
            <div className="bitfun-feedback__content-field">
              <Textarea
                label={t('feedback.content')}
                required
                value={content}
                disabled={submitting}
                onChange={event => handleContentChange(event.target.value)}
                placeholder={t('feedback.contentPlaceholder')}
                error={content.length > 0 && !content.trim()}
                errorMessage={t('feedback.errors.contentRequired')}
                data-testid="feedback-content"
              />
              <div className="bitfun-feedback__content-meta" aria-live="polite">
                <span>{wasTruncated ? t('feedback.contentTruncated') : ''}</span>
                <span>{contentLength}/2000</span>
              </div>
            </div>
            <div className="bitfun-feedback__correlation">
              <Switch
                checked={includeCorrelation}
                onChange={event => setIncludeCorrelation(event.target.checked)}
                disabled={submitting || !correlationAvailable}
                label={t('feedback.correlation.label')}
                description={correlationAvailable
                  ? t('feedback.correlation.description')
                  : t('feedback.correlation.unavailable')}
              />
            </div>
            <div className="bitfun-feedback__privacy">
              <Checkbox
                checked={privacyChecked}
                disabled={submitting}
                onChange={event => {
                  setPrivacyChecked(event.target.checked);
                  setSubmitError(null);
                }}
                label={t('feedback.privacyConsent')}
              />
              <Button
                type="button"
                variant="ghost"
                size="small"
                disabled={submitting}
                onClick={() => setShowPrivacy(true)}
              >
                {t('feedback.viewPrivacy')}
              </Button>
            </div>
            {submitError ? (
              <div className="bitfun-feedback__error" role="alert">
                {displayError(submitError)}
              </div>
            ) : null}
            {gitCodeError ? (
              <div className="bitfun-feedback__error" role="alert">
                {t('feedback.errors.gitcode')}
              </div>
            ) : null}
            <div className="bitfun-feedback__actions">
              <Button type="button" variant="ghost" onClick={openGitCode}>
                <ExternalLink size={15} aria-hidden="true" />
                {t('feedback.actions.gitcode')}
              </Button>
              <span className="bitfun-feedback__action-spacer" />
              <Button
                type="button"
                variant="secondary"
                disabled={submitting}
                onClick={requestClose}
              >
                {t('feedback.actions.cancel')}
              </Button>
              <Button
                type="submit"
                disabled={!canSubmit}
                isLoading={submitting}
                data-testid="feedback-submit"
              >
                <Send size={15} aria-hidden="true" />
                {submitError instanceof FeedbackApiError && submitError.retryable
                  ? t('feedback.actions.retry')
                  : t('feedback.actions.submit')}
              </Button>
            </div>
          </form>
        )}
      </Modal>
      <ConfirmDialog
        isOpen={showDiscardConfirm}
        onClose={() => setShowDiscardConfirm(false)}
        onConfirm={closeImmediately}
        title={t('feedback.discard.title')}
        message={t('feedback.discard.message')}
        confirmText={t('feedback.discard.confirm')}
        cancelText={t('feedback.discard.continue')}
        confirmDanger
      />
      <PrivacyStatementDialog
        isOpen={showPrivacy}
        onClose={() => setShowPrivacy(false)}
        variant="readonly"
      />
    </>
  );
};

function asFeedbackError(error: unknown): FeedbackApiError {
  return error instanceof FeedbackApiError
    ? error
    : new FeedbackApiError(
      'UNKNOWN_ERROR',
      'Feedback request could not be completed',
      true,
    );
}

export default FeedbackDialog;
