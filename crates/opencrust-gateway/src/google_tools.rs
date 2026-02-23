use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use opencrust_agents::tools::{Tool, ToolContext, ToolOutput};
use opencrust_common::{Error, Result};
use reqwest::Url;

const BASE64_URL_SAFE_NO_PAD: base64::engine::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";
const GOOGLE_GMAIL_API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users";
const GOOGLE_DRIVE_FILES_API: &str = "https://www.googleapis.com/drive/v3/files";
const DEFAULT_MAX_RESULTS: usize = 10;
const MAX_RESULTS_LIMIT: usize = 50;

#[derive(Debug, Clone)]
struct GoogleOAuthConfig {
    client_id: String,
    client_secret: String,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleTokenResponse {
    access_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleApiErrorBody {
    error: Option<GoogleApiError>,
    error_description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum GoogleApiError {
    Object { message: Option<String> },
    String(String),
}

#[derive(Debug, serde::Deserialize)]
struct GoogleCalendarEventsResponse {
    items: Option<Vec<GoogleCalendarEvent>>,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleCalendarEvent {
    id: Option<String>,
    status: Option<String>,
    summary: Option<String>,
    html_link: Option<String>,
    start: Option<GoogleCalendarEventDateTime>,
    end: Option<GoogleCalendarEventDateTime>,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleCalendarEventDateTime {
    date_time: Option<String>,
    date: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailMessagesResponse {
    messages: Option<Vec<GmailMessageRef>>,
    result_size_estimate: Option<u64>,
    next_page_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailMessageRef {
    id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DriveFilesResponse {
    files: Option<Vec<DriveFile>>,
    next_page_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct DriveFile {
    id: Option<String>,
    name: Option<String>,
    mime_type: Option<String>,
    web_view_link: Option<String>,
    modified_time: Option<String>,
    size: Option<String>,
}

#[derive(Debug)]
struct ListEventsInput {
    calendar_id: String,
    time_min: DateTime<Utc>,
    time_max: Option<DateTime<Utc>>,
    max_results: usize,
}

#[derive(Debug)]
struct ListGmailInput {
    user_id: String,
    query: Option<String>,
    max_results: usize,
}

#[derive(Debug)]
struct ListDriveFilesInput {
    query: Option<String>,
    order_by: Option<String>,
    max_results: usize,
}

#[derive(Debug)]
struct CalendarGetEventInput {
    calendar_id: String,
    event_id: String,
}

#[derive(Debug)]
struct GmailGetMessageInput {
    user_id: String,
    message_id: String,
    format: String,
}

#[derive(Debug)]
struct GmailSendMessageInput {
    user_id: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    thread_id: Option<String>,
}

#[derive(Debug)]
struct DriveGetFileMetadataInput {
    file_id: String,
    fields: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailSendResponse {
    id: Option<String>,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
}

/// Google Calendar tool backed by OAuth refresh token from secure storage.
pub struct GoogleCalendarListEventsTool {
    http: reqwest::Client,
}

impl GoogleCalendarListEventsTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn list_events(&self, input: &ListEventsInput) -> Result<Vec<serde_json::Value>> {
        let access_token = read_access_token(&self.http).await?;

        let mut url = Url::parse(GOOGLE_CALENDAR_API_BASE)
            .map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::Agent("failed to build calendar url path".to_string()))?;
            segments.push(&input.calendar_id);
            segments.push("events");
        }
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("singleEvents", "true");
            qp.append_pair("orderBy", "startTime");
            qp.append_pair("maxResults", &input.max_results.to_string());
            qp.append_pair("timeMin", &input.time_min.to_rfc3339());
            if let Some(time_max) = input.time_max {
                qp.append_pair("timeMax", &time_max.to_rfc3339());
            }
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("google calendar request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "google calendar list events failed ({status}): {details}"
            )));
        }

        let payload = resp
            .json::<GoogleCalendarEventsResponse>()
            .await
            .map_err(|e| Error::Agent(format!("failed to parse google calendar response: {e}")))?;

        Ok(payload
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|evt| {
                serde_json::json!({
                    "id": evt.id,
                    "status": evt.status,
                    "summary": evt.summary,
                    "start": event_time(evt.start),
                    "end": event_time(evt.end),
                    "html_link": evt.html_link,
                })
            })
            .collect())
    }
}

#[async_trait]
impl Tool for GoogleCalendarListEventsTool {
    fn name(&self) -> &str {
        "google_calendar_list_events"
    }

    fn description(&self) -> &str {
        "List upcoming Google Calendar events using the connected Google Workspace OAuth account."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "calendar_id": {
                    "type": "string",
                    "description": "Calendar ID to query. Use 'primary' for the main calendar.",
                    "default": "primary"
                },
                "time_min": {
                    "type": "string",
                    "description": "Start of query window. Accepts RFC3339, 'now', 'today', 'tomorrow', or YYYY-MM-DD. Defaults to now."
                },
                "time_max": {
                    "type": "string",
                    "description": "End of query window. Accepts RFC3339, 'now', 'today', 'tomorrow', or YYYY-MM-DD (optional)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum events to return (1-50). Defaults to 10."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_list_events_input(&input)?;
        let events = self.list_events(&parsed).await?;

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&serde_json::json!({
                "calendar_id": parsed.calendar_id,
                "time_min": parsed.time_min.to_rfc3339(),
                "time_max": parsed.time_max.map(|dt| dt.to_rfc3339()),
                "count": events.len(),
                "events": events,
            }))
            .unwrap_or_else(|_| "{\"error\":\"failed to serialize events\"}".to_string()),
        ))
    }
}

/// Google Calendar get-event tool backed by OAuth refresh token.
pub struct GoogleCalendarGetEventTool {
    http: reqwest::Client,
}

impl GoogleCalendarGetEventTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn get_event(&self, input: &CalendarGetEventInput) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url = Url::parse(GOOGLE_CALENDAR_API_BASE)
            .map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::Agent("failed to build calendar event url path".to_string()))?;
            segments.push(&input.calendar_id);
            segments.push("events");
            segments.push(&input.event_id);
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("google calendar get event request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "google calendar get event failed ({status}): {details}"
            )));
        }

        let payload = resp.json::<serde_json::Value>().await.map_err(|e| {
            Error::Agent(format!(
                "failed to parse google calendar event response: {e}"
            ))
        })?;

        Ok(payload)
    }
}

#[async_trait]
impl Tool for GoogleCalendarGetEventTool {
    fn name(&self) -> &str {
        "google_calendar_get_event"
    }

    fn description(&self) -> &str {
        "Get a single Google Calendar event by event ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "calendar_id": {
                    "type": "string",
                    "description": "Calendar ID that contains the event. Defaults to 'primary'.",
                    "default": "primary"
                },
                "event_id": {
                    "type": "string",
                    "description": "Google Calendar event ID."
                }
            },
            "required": ["event_id"]
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_calendar_get_event_input(&input)?;
        let payload = self.get_event(&parsed).await?;
        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| "{\"error\":\"failed to serialize event\"}".to_string()),
        ))
    }
}

/// Gmail read-only listing tool backed by OAuth refresh token.
pub struct GoogleGmailListMessagesTool {
    http: reqwest::Client,
}

impl GoogleGmailListMessagesTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn list_messages(&self, input: &ListGmailInput) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url =
            Url::parse(GOOGLE_GMAIL_API_BASE).map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::Agent("failed to build gmail url path".to_string()))?;
            segments.push(&input.user_id);
            segments.push("messages");
        }
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("maxResults", &input.max_results.to_string());
            if let Some(query) = input.query.as_deref() {
                qp.append_pair("q", query);
            }
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("gmail list request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "gmail list messages failed ({status}): {details}"
            )));
        }

        let payload = resp
            .json::<GmailMessagesResponse>()
            .await
            .map_err(|e| Error::Agent(format!("failed to parse gmail response: {e}")))?;

        let messages = payload
            .messages
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "thread_id": m.thread_id,
                })
            })
            .collect::<Vec<_>>();

        Ok(serde_json::json!({
            "user_id": input.user_id,
            "query": input.query,
            "count": messages.len(),
            "result_size_estimate": payload.result_size_estimate,
            "next_page_token": payload.next_page_token,
            "messages": messages,
        }))
    }
}

#[async_trait]
impl Tool for GoogleGmailListMessagesTool {
    fn name(&self) -> &str {
        "google_gmail_list_messages"
    }

    fn description(&self) -> &str {
        "List Gmail message IDs from the connected Google Workspace account."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Gmail user id, defaults to 'me'.",
                    "default": "me"
                },
                "query": {
                    "type": "string",
                    "description": "Optional Gmail search query, e.g. 'newer_than:7d from:billing@example.com'."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum messages to return (1-50). Defaults to 10."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_list_gmail_input(&input)?;
        let payload = self.list_messages(&parsed).await?;

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
                "{\"error\":\"failed to serialize gmail messages\"}".to_string()
            }),
        ))
    }
}

/// Gmail get-message tool backed by OAuth refresh token.
pub struct GoogleGmailGetMessageTool {
    http: reqwest::Client,
}

impl GoogleGmailGetMessageTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn get_message(&self, input: &GmailGetMessageInput) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url =
            Url::parse(GOOGLE_GMAIL_API_BASE).map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url.path_segments_mut().map_err(|_| {
                Error::Agent("failed to build gmail get message url path".to_string())
            })?;
            segments.push(&input.user_id);
            segments.push("messages");
            segments.push(&input.message_id);
        }
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("format", &input.format);
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("gmail get message request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "gmail get message failed ({status}): {details}"
            )));
        }

        let payload = resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| Error::Agent(format!("failed to parse gmail message response: {e}")))?;

        Ok(payload)
    }
}

#[async_trait]
impl Tool for GoogleGmailGetMessageTool {
    fn name(&self) -> &str {
        "google_gmail_get_message"
    }

    fn description(&self) -> &str {
        "Get a single Gmail message by message ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Gmail user id, defaults to 'me'.",
                    "default": "me"
                },
                "message_id": {
                    "type": "string",
                    "description": "Gmail message ID."
                },
                "format": {
                    "type": "string",
                    "description": "Gmail response format: 'minimal', 'metadata', or 'full'. Defaults to 'metadata'.",
                    "default": "metadata"
                }
            },
            "required": ["message_id"]
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_gmail_get_message_input(&input)?;
        let payload = self.get_message(&parsed).await?;
        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| "{\"error\":\"failed to serialize message\"}".to_string()),
        ))
    }
}

/// Gmail send-message tool backed by OAuth refresh token.
pub struct GoogleGmailSendMessageTool {
    http: reqwest::Client,
}

impl GoogleGmailSendMessageTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn send_message(&self, input: &GmailSendMessageInput) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url =
            Url::parse(GOOGLE_GMAIL_API_BASE).map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::Agent("failed to build gmail send url path".to_string()))?;
            segments.push(&input.user_id);
            segments.push("messages");
            segments.push("send");
        }

        let raw_message = build_gmail_raw_message(input);
        let encoded = BASE64_URL_SAFE_NO_PAD.encode(raw_message.as_bytes());
        let mut payload = serde_json::json!({ "raw": encoded });
        if let Some(thread_id) = input.thread_id.as_deref() {
            payload["threadId"] = serde_json::json!(thread_id);
        }

        let resp = self
            .http
            .post(url)
            .bearer_auth(access_token)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("gmail send request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let mut details = parse_google_error_body(&body).unwrap_or(body);
            let lower = details.to_ascii_lowercase();
            if lower.contains("insufficient authentication scopes")
                || lower.contains("insufficient permissions")
            {
                details.push_str(
                    ". Reconnect Google integration to grant the gmail.send OAuth scope.",
                );
            }
            return Err(Error::Agent(format!(
                "gmail send message failed ({status}): {details}"
            )));
        }

        let response = resp
            .json::<GmailSendResponse>()
            .await
            .map_err(|e| Error::Agent(format!("failed to parse gmail send response: {e}")))?;

        Ok(serde_json::json!({
            "user_id": input.user_id,
            "message_id": response.id,
            "thread_id": response.thread_id.or_else(|| input.thread_id.clone()),
            "label_ids": response.label_ids.unwrap_or_default(),
            "to": input.to.clone(),
            "cc": input.cc.clone(),
            "bcc_count": input.bcc.len(),
            "subject": input.subject,
        }))
    }
}

#[async_trait]
impl Tool for GoogleGmailSendMessageTool {
    fn name(&self) -> &str {
        "google_gmail_send_message"
    }

    fn description(&self) -> &str {
        "Send a Gmail message from the connected Google Workspace account."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "user_id": {
                    "type": "string",
                    "description": "Gmail user id, defaults to 'me'.",
                    "default": "me"
                },
                "to": {
                    "description": "Recipient email address or list of addresses.",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "cc": {
                    "description": "Optional CC email address(es).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "bcc": {
                    "description": "Optional BCC email address(es).",
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "subject": {
                    "type": "string",
                    "description": "Email subject line."
                },
                "body_text": {
                    "type": "string",
                    "description": "Plain-text body. Required unless body_html is provided."
                },
                "body_html": {
                    "type": "string",
                    "description": "Optional HTML body."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Optional Gmail thread ID to send as a reply in an existing thread."
                }
            },
            "required": ["to", "subject"]
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_gmail_send_message_input(&input)?;
        let payload = self.send_message(&parsed).await?;
        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| "{\"error\":\"failed to serialize sent message\"}".to_string()),
        ))
    }
}

/// Drive read-only listing tool backed by OAuth refresh token.
pub struct GoogleDriveListFilesTool {
    http: reqwest::Client,
}

impl GoogleDriveListFilesTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn list_files(&self, input: &ListDriveFilesInput) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url = Url::parse(GOOGLE_DRIVE_FILES_API)
            .map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("pageSize", &input.max_results.to_string());
            qp.append_pair(
                "fields",
                "nextPageToken,files(id,name,mimeType,webViewLink,modifiedTime,size)",
            );
            if let Some(query) = input.query.as_deref() {
                qp.append_pair("q", query);
            }
            if let Some(order_by) = input.order_by.as_deref() {
                qp.append_pair("orderBy", order_by);
            }
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("drive list request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "drive list files failed ({status}): {details}"
            )));
        }

        let payload = resp
            .json::<DriveFilesResponse>()
            .await
            .map_err(|e| Error::Agent(format!("failed to parse drive response: {e}")))?;

        let files = payload
            .files
            .unwrap_or_default()
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id,
                    "name": f.name,
                    "mime_type": f.mime_type,
                    "web_view_link": f.web_view_link,
                    "modified_time": f.modified_time,
                    "size": f.size,
                })
            })
            .collect::<Vec<_>>();

        Ok(serde_json::json!({
            "query": input.query,
            "order_by": input.order_by,
            "count": files.len(),
            "next_page_token": payload.next_page_token,
            "files": files,
        }))
    }
}

#[async_trait]
impl Tool for GoogleDriveListFilesTool {
    fn name(&self) -> &str {
        "google_drive_list_files"
    }

    fn description(&self) -> &str {
        "List Google Drive files from the connected Google Workspace account."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Optional Drive query, e.g. \"mimeType='application/pdf' and trashed=false\"."
                },
                "order_by": {
                    "type": "string",
                    "description": "Optional Drive orderBy string, e.g. 'modifiedTime desc'."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum files to return (1-50). Defaults to 10."
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_list_drive_input(&input)?;
        let payload = self.list_files(&parsed).await?;

        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| "{\"error\":\"failed to serialize drive files\"}".to_string()),
        ))
    }
}

/// Drive get-file-metadata tool backed by OAuth refresh token.
pub struct GoogleDriveGetFileMetadataTool {
    http: reqwest::Client,
}

impl GoogleDriveGetFileMetadataTool {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    async fn get_file_metadata(
        &self,
        input: &DriveGetFileMetadataInput,
    ) -> Result<serde_json::Value> {
        let access_token = read_access_token(&self.http).await?;

        let mut url = Url::parse(GOOGLE_DRIVE_FILES_API)
            .map_err(|e| Error::Agent(format!("bad url: {e}")))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::Agent("failed to build drive get file url path".to_string()))?;
            segments.push(&input.file_id);
        }
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair(
                "fields",
                input
                    .fields
                    .as_deref()
                    .unwrap_or("id,name,mimeType,webViewLink,modifiedTime,size,owners"),
            );
        }

        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| Error::Agent(format!("drive get file metadata request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let details = parse_google_error_body(&body).unwrap_or(body);
            return Err(Error::Agent(format!(
                "drive get file metadata failed ({status}): {details}"
            )));
        }

        let payload = resp.json::<serde_json::Value>().await.map_err(|e| {
            Error::Agent(format!("failed to parse drive file metadata response: {e}"))
        })?;

        Ok(payload)
    }
}

#[async_trait]
impl Tool for GoogleDriveGetFileMetadataTool {
    fn name(&self) -> &str {
        "google_drive_get_file_metadata"
    }

    fn description(&self) -> &str {
        "Get Google Drive file metadata by file ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_id": {
                    "type": "string",
                    "description": "Google Drive file ID."
                },
                "fields": {
                    "type": "string",
                    "description": "Optional Drive fields expression."
                }
            },
            "required": ["file_id"]
        })
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        input: serde_json::Value,
    ) -> Result<ToolOutput> {
        let parsed = parse_drive_get_file_metadata_input(&input)?;
        let payload = self.get_file_metadata(&parsed).await?;
        Ok(ToolOutput::success(
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| {
                "{\"error\":\"failed to serialize drive metadata\"}".to_string()
            }),
        ))
    }
}

async fn read_access_token(http: &reqwest::Client) -> Result<String> {
    let oauth = google_oauth_config().ok_or_else(|| {
        Error::Agent(
            "google oauth config missing. set GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET"
                .to_string(),
        )
    })?;
    let refresh_token = google_oauth_secret("GOOGLE_WORKSPACE_REFRESH_TOKEN").ok_or_else(|| {
        Error::Agent("google refresh token missing; reconnect integration".into())
    })?;

    let resp = http
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", oauth.client_id.as_str()),
            ("client_secret", oauth.client_secret.as_str()),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| Error::Agent(format!("google token request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let details = parse_google_error_body(&body).unwrap_or(body);
        return Err(Error::Agent(format!(
            "google token exchange failed ({status}): {details}"
        )));
    }

    let token = resp
        .json::<GoogleTokenResponse>()
        .await
        .map_err(|e| Error::Agent(format!("failed to parse google token response: {e}")))?;
    token
        .access_token
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| Error::Agent("google token response missing access_token".to_string()))
}

fn parse_list_events_input(input: &serde_json::Value) -> Result<ListEventsInput> {
    let calendar_id = input
        .get("calendar_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("primary")
        .to_string();

    let time_min = input
        .get("time_min")
        .and_then(|v| v.as_str())
        .map(parse_rfc3339_utc)
        .transpose()?
        .unwrap_or_else(Utc::now);

    let time_max = input
        .get("time_max")
        .and_then(|v| v.as_str())
        .map(parse_rfc3339_utc)
        .transpose()?;

    if let Some(max) = time_max
        && max <= time_min
    {
        return Err(Error::Agent(
            "time_max must be greater than time_min".to_string(),
        ));
    }

    let max_results = parse_max_results(input, DEFAULT_MAX_RESULTS);

    Ok(ListEventsInput {
        calendar_id,
        time_min,
        time_max,
        max_results,
    })
}

fn parse_calendar_get_event_input(input: &serde_json::Value) -> Result<CalendarGetEventInput> {
    let calendar_id = input
        .get("calendar_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("primary")
        .to_string();
    let event_id = input
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Agent("missing required 'event_id'".to_string()))?
        .to_string();

    Ok(CalendarGetEventInput {
        calendar_id,
        event_id,
    })
}

fn parse_list_gmail_input(input: &serde_json::Value) -> Result<ListGmailInput> {
    let user_id = input
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("me")
        .to_string();
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let max_results = parse_max_results(input, DEFAULT_MAX_RESULTS);

    Ok(ListGmailInput {
        user_id,
        query,
        max_results,
    })
}

fn parse_gmail_get_message_input(input: &serde_json::Value) -> Result<GmailGetMessageInput> {
    let user_id = input
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("me")
        .to_string();
    let message_id = input
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Agent("missing required 'message_id'".to_string()))?
        .to_string();
    let format = input
        .get("format")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("metadata")
        .to_ascii_lowercase();

    if !matches!(format.as_str(), "minimal" | "metadata" | "full") {
        return Err(Error::Agent(
            "invalid 'format'; use one of: minimal, metadata, full".to_string(),
        ));
    }

    Ok(GmailGetMessageInput {
        user_id,
        message_id,
        format,
    })
}

fn parse_gmail_send_message_input(input: &serde_json::Value) -> Result<GmailSendMessageInput> {
    let user_id = input
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("me")
        .to_string();

    let to = parse_gmail_recipients(input, "to", true)?;
    let cc = parse_gmail_recipients(input, "cc", false)?;
    let bcc = parse_gmail_recipients(input, "bcc", false)?;

    let subject = input
        .get("subject")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Agent("missing required 'subject'".to_string()))
        .and_then(|v| sanitize_gmail_header(v, "subject"))?;

    let body_text = input
        .get("body_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty());
    let body_html = input
        .get("body_html")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty());

    let Some(body_text) = body_text.or_else(|| body_html.clone()) else {
        return Err(Error::Agent(
            "missing message body: provide 'body_text' or 'body_html'".to_string(),
        ));
    };

    let thread_id = input
        .get("thread_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    Ok(GmailSendMessageInput {
        user_id,
        to,
        cc,
        bcc,
        subject,
        body_text,
        body_html,
        thread_id,
    })
}

fn parse_list_drive_input(input: &serde_json::Value) -> Result<ListDriveFilesInput> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let order_by = input
        .get("order_by")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    let max_results = parse_max_results(input, DEFAULT_MAX_RESULTS);

    Ok(ListDriveFilesInput {
        query,
        order_by,
        max_results,
    })
}

fn parse_drive_get_file_metadata_input(
    input: &serde_json::Value,
) -> Result<DriveGetFileMetadataInput> {
    let file_id = input
        .get("file_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Agent("missing required 'file_id'".to_string()))?
        .to_string();
    let fields = input
        .get("fields")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    Ok(DriveGetFileMetadataInput { file_id, fields })
}

fn parse_max_results(input: &serde_json::Value, default_value: usize) -> usize {
    input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default_value)
        .clamp(1, MAX_RESULTS_LIMIT)
}

fn parse_gmail_recipients(
    input: &serde_json::Value,
    key: &str,
    required: bool,
) -> Result<Vec<String>> {
    let Some(value) = input.get(key) else {
        if required {
            return Err(Error::Agent(format!("missing required '{key}'")));
        }
        return Ok(Vec::new());
    };

    let mut recipients = Vec::new();
    match value {
        serde_json::Value::String(raw) => {
            for item in raw.split(',') {
                if item.trim().is_empty() {
                    continue;
                }
                recipients.push(sanitize_gmail_header(item, key)?);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                let raw = item.as_str().ok_or_else(|| {
                    Error::Agent(format!("'{key}' must contain only string values"))
                })?;
                recipients.push(sanitize_gmail_header(raw, key)?);
            }
        }
        _ => {
            return Err(Error::Agent(format!(
                "'{key}' must be a string or array of strings"
            )));
        }
    }

    if required && recipients.is_empty() {
        return Err(Error::Agent(format!(
            "missing required '{key}' recipient address"
        )));
    }

    Ok(recipients)
}

fn sanitize_gmail_header(raw: &str, field: &str) -> Result<String> {
    let cleaned = raw.replace(['\r', '\n'], " ").trim().to_string();
    if cleaned.is_empty() {
        return Err(Error::Agent(format!("'{field}' cannot be empty")));
    }
    Ok(cleaned)
}

fn normalize_gmail_body(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .collect::<Vec<_>>()
        .join("\r\n")
}

fn build_gmail_raw_message(input: &GmailSendMessageInput) -> String {
    let mut message = String::new();
    message.push_str("To: ");
    message.push_str(&input.to.join(", "));
    message.push_str("\r\n");

    if !input.cc.is_empty() {
        message.push_str("Cc: ");
        message.push_str(&input.cc.join(", "));
        message.push_str("\r\n");
    }

    if !input.bcc.is_empty() {
        message.push_str("Bcc: ");
        message.push_str(&input.bcc.join(", "));
        message.push_str("\r\n");
    }

    message.push_str("Subject: ");
    message.push_str(&input.subject);
    message.push_str("\r\n");
    message.push_str("MIME-Version: 1.0\r\n");

    if let Some(body_html) = input.body_html.as_deref() {
        let boundary = format!("opencrust-{}", uuid::Uuid::new_v4().simple());
        message.push_str(&format!(
            "Content-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n"
        ));

        message.push_str(&format!("--{boundary}\r\n"));
        message.push_str("Content-Type: text/plain; charset=\"UTF-8\"\r\n");
        message.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
        message.push_str(&normalize_gmail_body(&input.body_text));
        message.push_str("\r\n");

        message.push_str(&format!("--{boundary}\r\n"));
        message.push_str("Content-Type: text/html; charset=\"UTF-8\"\r\n");
        message.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
        message.push_str(&normalize_gmail_body(body_html));
        message.push_str("\r\n");

        message.push_str(&format!("--{boundary}--\r\n"));
        return message;
    }

    message.push_str("Content-Type: text/plain; charset=\"UTF-8\"\r\n");
    message.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
    message.push_str(&normalize_gmail_body(&input.body_text));
    message.push_str("\r\n");
    message
}

fn parse_rfc3339_utc(value: &str) -> Result<DateTime<Utc>> {
    let raw = value.trim();
    if raw.is_empty() {
        return Err(Error::Agent("datetime value cannot be empty".to_string()));
    }

    match raw.to_ascii_lowercase().as_str() {
        "now" => return Ok(Utc::now()),
        "today" => {
            let today = Utc::now().date_naive();
            return Ok(DateTime::<Utc>::from_naive_utc_and_offset(
                today.and_hms_opt(0, 0, 0).ok_or_else(|| {
                    Error::Agent("failed to build datetime for 'today'".to_string())
                })?,
                Utc,
            ));
        }
        "tomorrow" => {
            let tomorrow = Utc::now().date_naive().succ_opt().ok_or_else(|| {
                Error::Agent("failed to build datetime for 'tomorrow'".to_string())
            })?;
            return Ok(DateTime::<Utc>::from_naive_utc_and_offset(
                tomorrow.and_hms_opt(0, 0, 0).ok_or_else(|| {
                    Error::Agent("failed to build datetime for 'tomorrow'".to_string())
                })?,
                Utc,
            ));
        }
        _ => {}
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(dt.with_timezone(&Utc));
    }

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(
            date.and_hms_opt(0, 0, 0).ok_or_else(|| {
                Error::Agent("failed to build datetime for YYYY-MM-DD input".to_string())
            })?,
            Utc,
        ));
    }

    Err(Error::Agent(
        "invalid datetime. use RFC3339 (2026-02-22T16:00:00Z), 'now', 'today', 'tomorrow', or YYYY-MM-DD"
            .to_string(),
    ))
}

fn event_time(value: Option<GoogleCalendarEventDateTime>) -> Option<String> {
    let dt = value?;
    if let Some(date_time) = dt.date_time {
        return Some(date_time);
    }
    dt.date
}

fn google_oauth_config() -> Option<GoogleOAuthConfig> {
    let client_id = google_oauth_secret("GOOGLE_CLIENT_ID")?;
    let client_secret = google_oauth_secret("GOOGLE_CLIENT_SECRET")?;

    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return None;
    }

    Some(GoogleOAuthConfig {
        client_id,
        client_secret,
    })
}

fn google_oauth_secret(key: &str) -> Option<String> {
    crate::bootstrap::default_vault_path()
        .and_then(|path| opencrust_security::try_vault_get(&path, key))
        .or_else(|| std::env::var(key).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .filter(|v| !looks_like_placeholder_secret(v))
}

fn looks_like_placeholder_secret(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("your_") && trimmed.ends_with("_here")
}

fn parse_google_error_body(body: &str) -> Option<String> {
    serde_json::from_str::<GoogleApiErrorBody>(body)
        .ok()
        .and_then(|payload| {
            payload.error_description.or(match payload.error {
                Some(GoogleApiError::Object { message }) => message,
                Some(GoogleApiError::String(text)) => Some(text),
                None => None,
            })
        })
}
