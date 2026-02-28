import { createGoogleIntegrationController } from '/assets/webchat/integrations/google.js';

const SERVICE_META = {
  calendar: { title: "Google Calendar", scopes: "calendar.readonly, calendar.events" },
  gmail: { title: "Gmail", scopes: "gmail.readonly, gmail.send" },
  drive: { title: "Google Drive", scopes: "drive.readonly" },
  sheets: { title: "Google Sheets", scopes: "spreadsheets" },
  contacts: { title: "Google Contacts", scopes: "contacts.readonly" },
};

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

  const modal = document.getElementById("integration-modal");
  const modalCloseBtn = document.getElementById("modal-close");
  const modalTitle = document.getElementById("modal-title");
  const googleConfigToggleBtn = document.getElementById("google-config-toggle");
  const googleIntegrationConfigEl = document.getElementById("google-integration-config");

  // Google integration controller (shared across all Google services)
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

  // ── Modal open/close ──

  function openModal(service) {
    if (!modal) return;
    const meta = SERVICE_META[service] || { title: service, scopes: "—" };
    if (modalTitle) modalTitle.textContent = meta.title;

    modal.classList.add("is-open");
    modal.setAttribute("aria-hidden", "false");
  }

  function closeModal() {
    if (!modal) return;
    modal.classList.remove("is-open");
    modal.setAttribute("aria-hidden", "true");
  }

  // Attach click to all Configure buttons
  document.querySelectorAll(".conn-configure").forEach((btn) => {
    btn.addEventListener("click", () => {
      const service = btn.getAttribute("data-service");
      openModal(service);
    });
  });

  if (modalCloseBtn) {
    modalCloseBtn.addEventListener("click", closeModal);
  }

  // Close on backdrop click
  if (modal) {
    modal.addEventListener("click", (e) => {
      if (e.target === modal) closeModal();
    });
  }

  // Config section toggle
  if (googleConfigToggleBtn && googleIntegrationConfigEl) {
    googleConfigToggleBtn.addEventListener("click", () => {
      const isHidden = googleIntegrationConfigEl.style.display === "none" || !googleIntegrationConfigEl.style.display;
      googleIntegrationConfigEl.style.display = isHidden ? "grid" : "none";
      googleConfigToggleBtn.textContent = isHidden ? "Hide Settings" : "Configure";
    });
  }

  // Close on Escape
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && modal?.classList.contains("is-open")) {
      closeModal();
    }
  });

  return {
    load: () => googleIntegration.load(),
  };
}
