import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const readSource = (relativePath: string): string =>
  readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8').replace(/\r\n?/g, '\n');

describe('OpenHarmony privacy policy management contract', () => {
  it('offers managed full, not-accepted, and read-only detail modes', () => {
    const dialog = readSource('./PrivacyStatementDialog.tsx');
    const about = readSource('../AboutDialog/AboutDialog.tsx');

    expect(dialog).toContain("variant?: 'about' | 'readonly'");
    expect(dialog).toContain("t('privacy.withdraw')");
    expect(dialog).toContain("t('privacy.enableFull')");
    expect(dialog).toContain("variant === 'about'");
    expect(about).toContain("setSubDialog('privacy')");
    expect(about).toContain('privacyStatus.hasUnreadUpdate');
  });

  it('keeps collection disabled when withdrawal persistence or full-mode application fails', () => {
    const native = readSource('../../../../../apps/desktop/src/api/privacy_api.rs');
    const dialog = readSource('./PrivacyStatementDialog.tsx');
    const gate = readSource('./PrivacyGate.tsx');

    const withdraw = native.slice(
      native.indexOf('pub async fn privacy_enter_not_accepted'),
      native.indexOf('pub async fn privacy_mark_viewed'),
    );
    expect(withdraw.indexOf('state.enter_not_accepted_mode()?')).toBeLessThan(
      withdraw.indexOf('.enter_not_accepted('),
    );
    expect(withdraw).not.toContain('suspend_for_privacy');
    expect(dialog).toContain("operationError === 'withdraw'");
    expect(dialog).toContain("operationError === 'apply'");
    expect(gate).toContain('applyRetryRequired');
    expect(gate).toContain("applyCollectionPolicy('full', locale)");
    expect(`${native}\n${dialog}`).not.toContain('quitApp');
  });

  it('preserves prior consent for editorial updates and records local viewing', () => {
    const service = readSource(
      '../../../../../crates/services/services-integrations/src/privacy/mod.rs',
    );

    expect(service).toContain('PrivacyChangeType::Editorial');
    expect(service).toContain('"2026.07.1-dev-placeholder"');
    expect(service).toContain('state.viewed_policy_version.as_deref() != Some(POLICY_VERSION)');
    expect(service).toContain('editorial_update_keeps_consent_and_clears_marker_after_viewing');
    expect(service).toContain('changed_consent_generation_requires_a_new_choice');
  });
});
