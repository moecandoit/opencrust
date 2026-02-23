export function createGoogleIntegrationController({
  statusEl,
  metaEl,
  toggleBtn,
  clientIdInputEl,
  clientSecretInputEl,
  redirectUriInputEl,
  saveBtn,
  diagnosticsEl,
  vaultStatusEl,
  feedbackEl,
  onSystemMessage,
  onErrorMessage,
}) {
  const canonicalLocalHost = window.location.hostname === "localhost" ? "localhost" : "127.0.0.1";
  const fallbackRedirectUri = `http://${canonicalLocalHost}:3000/api/integrations/google/callback`;
  let expectedRedirectUri = fallbackRedirectUri;
  let expectedOrigin = `http://${canonicalLocalHost}:3000`;
  let connected = false;
  let loading = false;
  let authConfigured = false;
  const defaultMeta = metaEl
    ? metaEl.textContent.trim()
    : "Connect your Google Workspace account.";

  const reportSystem = (message) => {
    if (typeof onSystemMessage === "function") onSystemMessage(message);
  };
  const reportError = (message) => {
    if (typeof onErrorMessage === "function") onErrorMessage(message);
  };

  function tryOriginFromUrl(value) {
    try {
      return new URL(value).origin;
    } catch {
      return null;
    }
  }

  function getExpectedRedirectUri() {
    const value = redirectUriInputEl ? redirectUriInputEl.value.trim() : "";
    return value || expectedRedirectUri || fallbackRedirectUri;
  }

  function getExpectedOrigin() {
    return (
      tryOriginFromUrl(getExpectedRedirectUri()) ||
      expectedOrigin ||
      window.location.origin
    );
  }

  async function parseApiResponse(resp) {
    const raw = await resp.text();
    if (!raw) {
      return { ok: resp.ok, status: resp.status, data: null, raw: "" };
    }
    try {
      return { ok: resp.ok, status: resp.status, data: JSON.parse(raw), raw };
    } catch {
      return { ok: resp.ok, status: resp.status, data: null, raw };
    }
  }

  function setFeedback(message, level = "info") {
    if (!feedbackEl) return;
    feedbackEl.classList.remove("is-info", "is-success", "is-error");
    feedbackEl.classList.add(
      level === "success" ? "is-success" : level === "error" ? "is-error" : "is-info"
    );
    feedbackEl.textContent = message || "";
  }

  function setVaultStatus(message, level = "info") {
    if (!vaultStatusEl) return;
    vaultStatusEl.textContent = message || "";
    vaultStatusEl.classList.remove("is-success", "is-error");
    if (level === "success") {
      vaultStatusEl.classList.add("is-success");
    } else if (level === "error") {
      vaultStatusEl.classList.add("is-error");
    }
  }

  function renderDiagnostics(data) {
    if (!diagnosticsEl) return;

    if (!data || typeof data !== "object") {
      diagnosticsEl.innerHTML = "";
      return;
    }

    const origin = data.authorized_js_origin || "(not derivable)";
    const redirect = data.authorized_redirect_uri || getExpectedRedirectUri();
    const issues = Array.isArray(data.issues) ? data.issues : [];
    const scopes = Array.isArray(data.required_oauth_scopes) ? data.required_oauth_scopes : [];

    const issueHtml = issues.length
      ? issues
          .map(
            (item) =>
              `<div class="diag-issue">- ${String(item)
                .replace(/&/g, "&amp;")
                .replace(/</g, "&lt;")
                .replace(/>/g, "&gt;")}</div>`
          )
          .join("")
      : '<div>Issues: none</div>';

    diagnosticsEl.innerHTML = `
      <div><strong>Authorized JavaScript origin:</strong> ${origin}</div>
      <div><strong>Authorized redirect URI:</strong> ${redirect}</div>
      <div><strong>Required OAuth scopes:</strong> ${scopes.length ? scopes.join(" ") : "(unavailable)"}</div>
      ${issueHtml}
    `;
  }

  function applySnapshot(snapshot) {
    connected = !!snapshot?.connected;
    authConfigured = !!snapshot?.auth_configured;
    const email = typeof snapshot?.email === "string" ? snapshot.email : "";

    if (statusEl) {
      statusEl.classList.remove("online", "offline");
      statusEl.classList.add(connected ? "online" : "offline");
      statusEl.textContent = connected ? "Connected" : "Not connected";
    }

    if (metaEl) {
      if (!authConfigured) {
        metaEl.textContent =
          "Set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET to enable Google OAuth connect.";
      } else if (connected && email) {
        metaEl.textContent = `Connected as ${email}. ${defaultMeta}`;
      } else {
        metaEl.textContent = defaultMeta;
      }
    }

    if (toggleBtn) {
      toggleBtn.classList.toggle("primary", !connected);
      toggleBtn.classList.toggle("btn-danger", connected);
      toggleBtn.textContent = connected ? "Disconnect" : "Connect";
    }
  }

  function setLoading(isLoading) {
    loading = !!isLoading;
    if (toggleBtn) {
      toggleBtn.disabled = loading;
      if (loading) {
        toggleBtn.textContent = "Updating...";
      } else {
        toggleBtn.textContent = connected ? "Disconnect" : "Connect";
      }
    }
    if (saveBtn) {
      saveBtn.disabled = loading;
    }
  }

  function setConfigInputsEnabled(enabled) {
    const disabled = !enabled;
    if (clientIdInputEl) clientIdInputEl.disabled = disabled;
    if (clientSecretInputEl) clientSecretInputEl.disabled = disabled;
    if (redirectUriInputEl) redirectUriInputEl.disabled = disabled;
    if (saveBtn) saveBtn.disabled = disabled;
  }

  async function loadStatus() {
    if (!statusEl || !toggleBtn) return;
    setLoading(true);
    setFeedback("Checking Google integration status...", "info");
    try {
      const resp = await fetch("/api/integrations/google");
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (!parsed.ok || !parsed.data) {
        throw new Error("Google integration endpoint unavailable or returned non-JSON.");
      }
      applySnapshot(data);
      if (data.connected) {
        setFeedback("Google Workspace is connected.", "success");
      } else if (data.auth_configured) {
        setFeedback("Ready to connect Google Workspace.", "info");
      } else {
        setFeedback("Enter Client ID and Client Secret, then save.", "info");
      }
    } catch (e) {
      applySnapshot({ connected: false, auth_configured: false });
      if (statusEl) {
        statusEl.classList.remove("online");
        statusEl.classList.add("offline");
        statusEl.textContent = "Unavailable";
      }
      setFeedback(`Could not reach gateway integration API: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  }

  async function loadConfigMeta() {
    try {
      const resp = await fetch("/api/integrations/google/config");
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (!parsed.ok) {
        setFeedback(data.message || "Failed to load OAuth config.", "error");
        reportError(data.message || "Failed to load Google OAuth config.");
        return;
      }
      if (!parsed.data) {
        setFeedback("Failed to load OAuth config: server returned non-JSON response.", "error");
        reportError("Failed to load Google OAuth config: non-JSON response.");
        return;
      }
      if (clientIdInputEl) {
        const configuredClientId = typeof data.client_id === "string" ? data.client_id : "";
        if (configuredClientId.includes("*")) {
          clientIdInputEl.value = "";
          clientIdInputEl.placeholder = `Configured: ${configuredClientId}`;
        } else {
          clientIdInputEl.value = configuredClientId;
          clientIdInputEl.placeholder = "Paste OAuth client ID";
        }
      }
      const configuredRedirect = typeof data.redirect_uri === "string" && data.redirect_uri.trim()
        ? data.redirect_uri.trim()
        : fallbackRedirectUri;
      expectedRedirectUri = configuredRedirect;
      expectedOrigin = tryOriginFromUrl(configuredRedirect) || window.location.origin;
      if (redirectUriInputEl) {
        redirectUriInputEl.value = configuredRedirect;
      }
    } catch (e) {
      setFeedback(`Failed to load OAuth config: ${e}`, "error");
      reportError(`Failed to load Google OAuth config: ${e}`);
      if (redirectUriInputEl && !redirectUriInputEl.value.trim()) {
        redirectUriInputEl.value = getExpectedRedirectUri();
      }
    }
  }

  async function loadDiagnostics() {
    if (!diagnosticsEl) return;
    try {
      const resp = await fetch("/api/integrations/google/diagnostics");
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (parsed.status === 404) {
        renderDiagnostics({
          authorized_js_origin: getExpectedOrigin(),
          authorized_redirect_uri: getExpectedRedirectUri(),
          issues: [
            "Diagnostics endpoint is unavailable on this gateway build. The running server is likely an older build and needs a restart.",
          ],
        });
        return;
      }
      if (!parsed.ok) {
        renderDiagnostics({
          authorized_js_origin: getExpectedOrigin(),
          authorized_redirect_uri: getExpectedRedirectUri(),
          issues: [data.message || "Failed to load diagnostics."],
        });
        return;
      }
      if (!parsed.data) {
        renderDiagnostics({
          authorized_js_origin: getExpectedOrigin(),
          authorized_redirect_uri: getExpectedRedirectUri(),
          issues: ["Diagnostics endpoint returned non-JSON response."],
        });
        return;
      }
      renderDiagnostics(data);
    } catch (e) {
      renderDiagnostics({
        authorized_js_origin: getExpectedOrigin(),
        authorized_redirect_uri: getExpectedRedirectUri(),
        issues: [`Diagnostics unavailable: ${e}`],
      });
    }
  }

  async function loadVaultStatus() {
    if (!vaultStatusEl) return;
    try {
      const resp = await fetch("/api/security/vault");
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (parsed.status === 404) {
        setVaultStatus("Secure persistence status endpoint is unavailable on this gateway build.", "error");
        return;
      }
      if (!parsed.ok) {
        setVaultStatus(data.message || "Could not read secure persistence status.", "error");
        return;
      }
      if (!parsed.data) {
        setVaultStatus("Secure persistence status endpoint returned non-JSON response.", "error");
        return;
      }
      if (data.unlocked) {
        setVaultStatus("Secure persistence is available on this gateway.", "success");
      } else if (data.vault_exists) {
        setVaultStatus(
          "Vault exists but passphrase is unavailable to this process. Configure OS keychain integration or server env passphrase.",
          "error"
        );
      } else {
        setVaultStatus(
          "Secure vault not initialized yet. First save will auto-initialize if OS keychain is available.",
          "info"
        );
      }
    } catch (e) {
      setVaultStatus(`Secure persistence status unavailable: ${e}`, "error");
    }
  }

  function hasDraftConfig() {
    const clientId = clientIdInputEl ? clientIdInputEl.value.trim() : "";
    const clientSecret = clientSecretInputEl
      ? clientSecretInputEl.value.trim()
      : "";
    return !!clientId && !!clientSecret;
  }

  async function saveConfig({ quiet = false } = {}) {
    const clientId = clientIdInputEl ? clientIdInputEl.value.trim() : "";
    const clientSecret = clientSecretInputEl
      ? clientSecretInputEl.value.trim()
      : "";
    const redirectUri = redirectUriInputEl
      ? redirectUriInputEl.value.trim() || getExpectedRedirectUri()
      : getExpectedRedirectUri();

    if (!clientId || !clientSecret) {
      setFeedback("Client ID and Client Secret are required.", "error");
      reportError("Client ID and Client Secret are required.");
      return false;
    }

    setLoading(true);
    setConfigInputsEnabled(false);
    setFeedback("Saving OAuth configuration...", "info");
    try {
      const resp = await fetch("/api/integrations/google/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          client_id: clientId,
          client_secret: clientSecret,
          redirect_uri: redirectUri,
        }),
      });
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (!parsed.ok) {
        setFeedback(data.message || "Failed to save OAuth config.", "error");
        reportError(data.message || "Failed to save Google OAuth config.");
        return false;
      }
      if (!parsed.data) {
        setFeedback("Failed to save OAuth config: server returned non-JSON response.", "error");
        reportError("Failed to save Google OAuth config: non-JSON response.");
        return false;
      }
      if (clientSecretInputEl) clientSecretInputEl.value = "";
      if (redirectUriInputEl) redirectUriInputEl.value = redirectUri;
      expectedRedirectUri = redirectUri;
      expectedOrigin = tryOriginFromUrl(redirectUri) || expectedOrigin;
      const persisted = data.persisted !== false;
      setFeedback(
        persisted
          ? "OAuth configuration saved and persisted."
          : "OAuth configuration saved in memory only. This server could not access secure persistence (OS keychain/env).",
        persisted ? "success" : "error"
      );
      if (!quiet) reportSystem("Google OAuth configuration saved.");
      await load();
      return true;
    } catch (e) {
      setFeedback(`Failed to save OAuth config: ${e}`, "error");
      reportError(`Failed to save Google OAuth config: ${e}`);
      return false;
    } finally {
      setConfigInputsEnabled(true);
      setLoading(false);
    }
  }

  async function load() {
    await loadStatus();
    await loadConfigMeta();
    await loadDiagnostics();
    await loadVaultStatus();
  }

  async function startOAuthConnect() {
    if (!authConfigured) {
      setFeedback(
        "Set and save Client ID/Secret before connecting Google Workspace.",
        "error"
      );
      reportError(
        "Google OAuth is not configured. Set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET."
      );
      return;
    }

    setLoading(true);
    setFeedback("Validating OAuth configuration...", "info");

    let authUrl = "";
    try {
      const resp = await fetch("/api/integrations/google/connect-url");
      const parsed = await parseApiResponse(resp);
      if (parsed.status === 404) {
        // Backward-compatible fallback for older gateway binaries.
        setFeedback(
          "connect-url endpoint unavailable. The running server is likely an older build and needs a restart. Falling back to /connect.",
          "info"
        );
        reportSystem(
          "connect-url endpoint unavailable on this running build; restart the gateway after updating."
        );
        authUrl = "/api/integrations/google/connect";
      } else {
        const data = parsed.data || {};
        if (!parsed.ok || !data.url) {
          setFeedback(data.message || "OAuth configuration is invalid.", "error");
          reportError(data.message || "OAuth configuration is invalid.");
          setLoading(false);
          return;
        }
        authUrl = data.url;
      }
    } catch (e) {
      setFeedback(`Failed to start OAuth flow: ${e}`, "error");
      reportError(`Failed to start Google OAuth flow: ${e}`);
      setLoading(false);
      return;
    }

    setFeedback("Opening Google consent screen...", "info");
    const popup = window.open(authUrl, "opencrust-google-oauth", "popup,width=520,height=720");

    if (!popup) {
      setFeedback("Popup blocked. Redirecting this tab to Google consent...", "info");
      setLoading(false);
      window.location.assign(authUrl);
      return;
    }

    setLoading(false);
    const startedAt = Date.now();
    const poll = setInterval(async () => {
      const timedOut = Date.now() - startedAt > 180000;
      if (!popup.closed && !timedOut) return;
      clearInterval(poll);
      await load();
      setLoading(false);
      if (timedOut) {
        setFeedback("OAuth window timed out. Try Connect again.", "error");
      }
    }, 1200);
  }

  async function disconnect() {
    setLoading(true);
    try {
      const resp = await fetch("/api/integrations/google/disconnect", {
        method: "POST",
      });
      const parsed = await parseApiResponse(resp);
      const data = parsed.data || {};
      if (!parsed.ok) {
        setFeedback(data.message || "Failed to disconnect Google integration.", "error");
        reportError(data.message || "Failed to disconnect Google integration.");
        return;
      }
      if (!parsed.data) {
        setFeedback("Failed to disconnect Google integration: non-JSON response.", "error");
        reportError("Failed to disconnect Google integration: non-JSON response.");
        return;
      }
      applySnapshot(data);
      setFeedback("Google Workspace disconnected.", "success");
      reportSystem("Google Workspace disconnected.");
    } catch (e) {
      setFeedback(`Failed to disconnect: ${e}`, "error");
      reportError(`Failed to disconnect Google integration: ${e}`);
    } finally {
      setLoading(false);
    }
  }

  async function toggle() {
    if (loading) return;
    if (!connected) {
      if (!authConfigured && hasDraftConfig()) {
        const ok = await saveConfig({ quiet: true });
        if (!ok) return;
      }
      await startOAuthConnect();
      return;
    }
    await disconnect();
  }

  async function onOAuthWindowMessage(event) {
    if (event.origin !== window.location.origin) return;
    if (!event.data || event.data.type !== "opencrust.google.oauth") return;
    const message =
      typeof event.data.message === "string" && event.data.message.trim()
        ? event.data.message
        : null;
    await load();
    if (event.data.success) {
      setFeedback(message || "Google Workspace connected.", "success");
      reportSystem("Google Workspace connected.");
    } else {
      setFeedback(message || "Google Workspace authorization failed.", "error");
      reportError(message || "Google Workspace authorization failed.");
    }
  }

  const onToggleClick = async () => {
    await toggle();
  };
  const onSaveClick = async () => {
    await saveConfig();
  };

  function attach() {
    if (toggleBtn) {
      toggleBtn.addEventListener("click", onToggleClick);
    }
    if (saveBtn) {
      saveBtn.addEventListener("click", onSaveClick);
    }
    window.addEventListener("message", onOAuthWindowMessage);
  }

  function detach() {
    if (toggleBtn) {
      toggleBtn.removeEventListener("click", onToggleClick);
    }
    if (saveBtn) {
      saveBtn.removeEventListener("click", onSaveClick);
    }
    window.removeEventListener("message", onOAuthWindowMessage);
  }

  return {
    attach,
    detach,
    load,
    toggle,
  };
}
