import { api } from './ApiClient';
import { createTauriCommandError } from '../errors/TauriCommandError';
import enUsPolicy from '../../../../../crates/services/services-integrations/src/privacy/assets/en-US.md?raw';
import zhCnPolicy from '../../../../../crates/services/services-integrations/src/privacy/assets/zh-CN.md?raw';

export type PrivacyLifecycleState =
  | 'choice_required'
  | 'full'
  | 'privacy_not_accepted'
  | 'resource_error';
export type PrivacyEffectiveMode = 'full' | 'privacy_not_accepted';
export type PrivacyChangeType = 'material' | 'editorial';

export interface PrivacyConsentRecord {
  consentVersion: string;
  acceptedPolicyVersion: string;
  acceptedDocumentSha256: string;
  acceptedAt: string;
  locale: string;
  appVersion: string;
}

export interface PrivacyPolicyView {
  policyVersion: string;
  consentVersion: string;
  changeType: PrivacyChangeType;
  effectiveAt: string;
  updatedAt: string;
  locale: string;
  documentSha256: string;
  content: string;
}

export interface PrivacyStatus {
  enabled: boolean;
  lifecycleState: PrivacyLifecycleState;
  effectiveMode: PrivacyEffectiveMode;
  releaseReady: boolean;
  hasUnreadUpdate: boolean;
  policy?: PrivacyPolicyView;
  consent?: PrivacyConsentRecord;
  configurationError?: string;
}

export interface AcceptPrivacyRequest {
  policyVersion: string;
  consentVersion: string;
  documentSha256: string;
  locale: string;
}

export const disabledPrivacyStatus: PrivacyStatus = {
  enabled: false,
  lifecycleState: 'full',
  effectiveMode: 'full',
  releaseReady: true,
  hasUnreadUpdate: false,
};

const bundledPreviewPolicies = {
  'zh-CN': {
    content: zhCnPolicy,
    documentSha256: '9164815a22b2b2021039a19ed6e92556ce6ea44e42dd0103869b7c0887ae48bb',
  },
  'en-US': {
    content: enUsPolicy,
    documentSha256: '71c9914ad977ff12fa31a5e228192d806b3b3d366498100bff615739d9b4c451',
  },
} as const;

export function bundledPrivacyPreviewStatus(locale: string): PrivacyStatus {
  const normalized = locale.toLowerCase().replace('_', '-');
  const policyLocale = normalized.startsWith('zh') ? 'zh-CN' : 'en-US';
  const policy = bundledPreviewPolicies[policyLocale];
  return {
    ...disabledPrivacyStatus,
    policy: {
      policyVersion: '2026.07.2-dev-placeholder',
      consentVersion: 'dev-placeholder-1',
      changeType: 'editorial',
      effectiveAt: '2026-07-22T00:00:00Z',
      updatedAt: '2026-07-28T00:00:00Z',
      locale: policyLocale,
      documentSha256: policy.documentSha256,
      content: policy.content,
    },
  };
}

export class PrivacyAPI {
  async initialize(): Promise<PrivacyStatus> {
    return this.invoke('privacy_initialize', {});
  }

  async getStatus(locale: string): Promise<PrivacyStatus> {
    return this.invoke('privacy_get_status', { locale });
  }

  async accept(request: AcceptPrivacyRequest): Promise<PrivacyStatus> {
    return this.invoke('privacy_accept', request);
  }

  async enterNotAccepted(locale: string): Promise<PrivacyStatus> {
    return this.invoke('privacy_enter_not_accepted', { locale });
  }

  async markViewed(policyVersion: string, locale: string): Promise<PrivacyStatus> {
    return this.invoke('privacy_mark_viewed', { policyVersion, locale });
  }

  async applyCollectionPolicy(
    mode: PrivacyEffectiveMode,
    locale: string,
  ): Promise<PrivacyStatus> {
    return this.invoke('privacy_apply_collection_policy', { mode, locale });
  }

  async showGateWindow(): Promise<void> {
    await api.invoke('show_main_window', {});
  }

  private async invoke<T>(command: string, request: object): Promise<T> {
    try {
      return await api.invoke<T>(command, { request }, { retries: 0 });
    } catch (error) {
      throw createTauriCommandError(command, error);
    }
  }
}

export const privacyAPI = new PrivacyAPI();
