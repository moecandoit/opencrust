import { createGoogleIntegrationController } from '/assets/webchat/integrations/google.js';

export function initIntegrationsView({ onSystemMessage, onErrorMessage }) {
  const googleIntegrationStatusEl = document.getElementById("google-integration-status");
  const googleIntegrationMetaEl = document.getElementById("google-integration-meta");
  const googleIntegrationToggleBtn = document.getElementById("google-integration-toggle");
  const googleClientIdInputEl = document.getElementById("google-client-id-input");
  const googleClientSecretInputEl = document.getElementById("google-client-secret-input");
  const googleRedirectUriInputEl = document.getElementById("google-redirect-uri-input");
  const googleIntegrationSaveBtn = document.getElementById("google-integration-save");
  const googleIntegrationFeedbackEl = document.getElementById("google-integration-feedback");
  const googleVaultStatusEl = document.getElementById("google-vault-status");
  const googleConfigToggleBtn = document.getElementById("google-config-toggle");
  const googleIntegrationConfigEl = document.getElementById("google-integration-config");

  const googleIntegration = createGoogleIntegrationController({
    statusEl: googleIntegrationStatusEl,
    metaEl: googleIntegrationMetaEl,
    toggleBtn: googleIntegrationToggleBtn,
    clientIdInputEl: googleClientIdInputEl,
    clientSecretInputEl: googleClientSecretInputEl,
    redirectUriInputEl: googleRedirectUriInputEl,
    saveBtn: googleIntegrationSaveBtn,
    vaultStatusEl: googleVaultStatusEl,
    feedbackEl: googleIntegrationFeedbackEl,
    onSystemMessage,
    onErrorMessage,
  });

  googleIntegration.attach();

  if (googleConfigToggleBtn && googleIntegrationConfigEl) {
    googleConfigToggleBtn.addEventListener("click", () => {
      const isHidden = googleIntegrationConfigEl.style.display === "none" || !googleIntegrationConfigEl.style.display;
      googleIntegrationConfigEl.style.display = isHidden ? "grid" : "none";
      googleConfigToggleBtn.textContent = isHidden ? "Hide Settings" : "Configure";
    });
  }

  return {
    load: () => googleIntegration.load(),
  };
}
