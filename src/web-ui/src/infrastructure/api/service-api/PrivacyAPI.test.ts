import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('./ApiClient', () => ({ api: { invoke } }));

import { bundledPrivacyPreviewStatus, PrivacyAPI } from './PrivacyAPI';

describe('PrivacyAPI', () => {
  beforeEach(() => invoke.mockReset());

  it('uses structured requests for every privacy transition', async () => {
    invoke.mockResolvedValue({
      enabled: true,
      lifecycleState: 'privacy_not_accepted',
      effectiveMode: 'privacy_not_accepted',
    });
    const client = new PrivacyAPI();
    await client.enterNotAccepted('zh-TW');
    await client.applyCollectionPolicy('privacy_not_accepted', 'zh-TW');
    expect(invoke).toHaveBeenNthCalledWith(
      1,
      'privacy_enter_not_accepted',
      { request: { locale: 'zh-TW' } },
      { retries: 0 },
    );
    expect(invoke).toHaveBeenNthCalledWith(
      2,
      'privacy_apply_collection_policy',
      { request: { mode: 'privacy_not_accepted', locale: 'zh-TW' } },
      { retries: 0 },
    );
  });

  it('falls traditional Chinese previews back to the simplified document', () => {
    const status = bundledPrivacyPreviewStatus('zh-Hant-HK');
    expect(status.enabled).toBe(false);
    expect(status.lifecycleState).toBe('full');
    expect(status.policy?.locale).toBe('zh-CN');
    expect(status.policy?.content).toContain('开发测试占位版');
    expect(status.policy?.policyVersion).toBe('2026.07.2-dev-placeholder');
    expect(status.policy?.changeType).toBe('editorial');
  });
});
