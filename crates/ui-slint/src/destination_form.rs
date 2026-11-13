use std::sync::{Arc, Mutex, MutexGuard};

use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use snapline_services::dto::upload::{
    DestinationHeaderInput, DestinationInput, DestinationKindDto, DestinationReq, DestinationResp,
    DestinationTestStatusDto, DestinationViewDto,
};

use crate::backend::{destination_kind, spawn_bg, Backend};
use crate::{DestinationForm, FormHeaderData, SnaplineWindow, UploadsState};

const CUSTOM_DESCRIPTION: &str =
    "Compatible with ShareX .sxcu files. Response values use JSON paths.";
const S3_PUBLIC_URL_PATTERN: &str = "https://<bucket>.s3.<region>.amazonaws.com/<key>";
const URL_PATTERN_ERROR: &str = "The public URL pattern must contain <key>.";

#[derive(Clone, Default)]
struct HeaderDraft {
    name: String,
    value: String,
    saved: bool,
}

#[derive(Default)]
struct FormState {
    id: Option<String>,
    kind: Option<DestinationKindDto>,
    imported_from_sxcu: bool,
    headers: Vec<HeaderDraft>,
    seq: u64,
}

type Shared = Arc<Mutex<FormState>>;

fn lock_form(form: &Shared) -> MutexGuard<'_, FormState> {
    match form.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub fn wire(app: &SnaplineWindow, backend: RcBackend) {
    let form: Shared = Arc::default();
    let weak = app.as_weak();
    let u = app.global::<UploadsState>();

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    u.on_add_destination(move |kind| {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if u.get_busy() || g.get_busy() {
            return;
        }
        let parsed = match destination_kind(&kind) {
            Ok(kind) => kind,
            Err(error) => {
                report(&w, &format!("{error:#}"));
                return;
            }
        };
        let seq = {
            let mut state = lock_form(&f);
            state.seq += 1;
            state.seq
        };
        u.set_busy(true);
        g.set_busy(true);
        u.set_destination_error("".into());
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| bg.destination_request(DestinationReq::Add { kind: parsed }),
            move |ui_bg, result| {
                if let Some(app) = w2.upgrade() {
                    app.global::<UploadsState>().set_busy(false);
                    app.global::<DestinationForm>().set_busy(false);
                }
                match result {
                    Ok(DestinationResp::Destination(view)) => {
                        let is_current = lock_form(&f2).seq == seq;
                        if is_current {
                            open(&w2, &f2, &view);
                        }
                        refresh(&w2, &ui_bg);
                    }
                    Ok(other) => tracing::warn!("unexpected add response: {other:?}"),
                    Err(error) => report(&w2, &format!("{error:#}")),
                }
            },
        );
    });

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    u.on_import_sxcu(move || {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if u.get_busy() || g.get_busy() {
            return;
        }
        let seq = {
            let mut state = lock_form(&f);
            state.seq += 1;
            state.seq
        };
        u.set_busy(true);
        g.set_busy(true);
        u.set_destination_error("".into());
        set_import_warning(&w, "");
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| bg.destination_request(DestinationReq::ImportSxcu),
            move |ui_bg, result| {
                if let Some(app) = w2.upgrade() {
                    app.global::<UploadsState>().set_busy(false);
                    app.global::<DestinationForm>().set_busy(false);
                }
                match result {
                    Ok(DestinationResp::Imported {
                        destination,
                        warnings,
                    }) => {
                        set_import_warning(&w2, &warnings.join("\n"));
                        if let Some(view) = destination {
                            let is_current = lock_form(&f2).seq == seq;
                            if is_current {
                                open(&w2, &f2, &view);
                            }
                        }
                        refresh(&w2, &ui_bg);
                    }
                    Ok(other) => tracing::warn!("unexpected import response: {other:?}"),
                    Err(error) => report(&w2, &format!("{error:#}")),
                }
            },
        );
    });

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    u.on_open_destination(move |id| {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        if u.get_busy() {
            return;
        }
        if u.get_active_id() == id {
            close_form(&w, &f);
            return;
        }
        lock_form(&f).seq += 1;
        match find(&b, &id) {
            Some(view) => open(&w, &f, &view),
            None => report(&w, "The upload destination was not found."),
        }
    });

    let (w, f) = (weak.clone(), form.clone());
    u.on_close_destination(move || {
        let Some(app) = w.upgrade() else { return };
        if app.global::<DestinationForm>().get_busy() {
            return;
        }
        close_form(&w, &f);
    });

    let g = app.global::<DestinationForm>();

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    g.on_save_destination(move || {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if g.get_busy() || u.get_busy() || g.get_test_state() == "testing" {
            return;
        }
        let (built, seq) = {
            let state = lock_form(&f);
            (build_input(&g, &state), state.seq)
        };
        let input = match built {
            Ok(input) => input,
            Err(message) => {
                g.set_error(message.into());
                return;
            }
        };
        g.set_busy(true);
        u.set_busy(true);
        g.set_error("".into());
        g.set_notice("".into());
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| bg.destination_request(DestinationReq::Save { input }),
            move |ui_bg, result| {
                let Some(app) = w2.upgrade() else { return };
                let u = app.global::<UploadsState>();
                let g = app.global::<DestinationForm>();
                g.set_busy(false);
                u.set_busy(false);
                match result {
                    Ok(DestinationResp::Destination(view)) => {
                        {
                            let mut state = lock_form(&f2);
                            if state.seq == seq {
                                fill(&g, &mut state, &view);
                                g.set_notice("Saved".into());
                            }
                        }
                        refresh(&w2, &ui_bg);
                    }
                    Ok(other) => tracing::warn!("unexpected save response: {other:?}"),
                    Err(error) => {
                        if lock_form(&f2).seq == seq {
                            g.set_error(format!("{error:#}").into());
                        }
                    }
                }
            },
        );
    });

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    g.on_test_destination(move || {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if g.get_busy() || u.get_busy() || g.get_test_state() == "testing" {
            return;
        }
        let (id, seq) = {
            let state = lock_form(&f);
            let Some(id) = state.id.clone() else { return };
            (id, state.seq)
        };
        g.set_busy(true);
        u.set_busy(true);
        g.set_error("".into());
        g.set_test_state("testing".into());
        g.set_test_message("".into());
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| {
                let resp = bg.destination_request(DestinationReq::Test { id });
                let copied = match &resp {
                    Ok(DestinationResp::Test { outcome })
                        if outcome.status == DestinationTestStatusDto::Succeeded =>
                    {
                        outcome
                            .link
                            .as_deref()
                            .filter(|link| !link.is_empty())
                            .is_some_and(|link| bg.host().copy_text(link).is_ok())
                    }
                    _ => false,
                };
                (resp, copied)
            },
            move |_, (result, copied)| {
                let Some(app) = w2.upgrade() else { return };
                let u = app.global::<UploadsState>();
                let g = app.global::<DestinationForm>();
                g.set_busy(false);
                u.set_busy(false);
                if lock_form(&f2).seq != seq {
                    return;
                }
                match result {
                    Ok(DestinationResp::Test { outcome })
                        if outcome.status == DestinationTestStatusDto::Succeeded =>
                    {
                        g.set_test_state("success".into());
                        g.set_test_message(
                            if copied {
                                "Test upload succeeded · link copied"
                            } else {
                                "Test upload succeeded"
                            }
                            .into(),
                        );
                    }
                    Ok(DestinationResp::Test { outcome }) => {
                        g.set_test_state("failed".into());
                        g.set_test_message(
                            outcome
                                .message
                                .unwrap_or_else(|| "The test upload failed.".into())
                                .into(),
                        );
                    }
                    Ok(other) => {
                        tracing::warn!("unexpected test response: {other:?}");
                        g.set_test_state("idle".into());
                    }
                    Err(error) => {
                        g.set_test_state("failed".into());
                        g.set_test_message(format!("{error:#}").into());
                    }
                }
            },
        );
    });

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    g.on_delete_destination(move || {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if g.get_busy() || u.get_busy() || g.get_test_state() == "testing" {
            return;
        }
        g.set_confirm_delete(false);
        let (id, seq) = {
            let mut state = lock_form(&f);
            let Some(id) = state.id.clone() else { return };
            state.seq += 1;
            (id, state.seq)
        };
        g.set_busy(true);
        u.set_busy(true);
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| bg.destination_request(DestinationReq::Remove { id }),
            move |ui_bg, result| {
                let Some(app) = w2.upgrade() else { return };
                let u = app.global::<UploadsState>();
                let g = app.global::<DestinationForm>();
                g.set_busy(false);
                u.set_busy(false);
                match result {
                    Ok(_) => {
                        let is_current = lock_form(&f2).seq == seq;
                        if is_current {
                            close_form(&w2, &f2);
                        }
                        refresh(&w2, &ui_bg);
                    }
                    Err(error) => {
                        if lock_form(&f2).seq == seq {
                            g.set_error(format!("{error:#}").into());
                        }
                    }
                }
            },
        );
    });

    let (b, w, f) = (backend.clone(), weak.clone(), form.clone());
    g.on_export_sxcu(move || {
        let Some(app) = w.upgrade() else { return };
        let u = app.global::<UploadsState>();
        let g = app.global::<DestinationForm>();
        if g.get_busy() || u.get_busy() || g.get_test_state() == "testing" {
            return;
        }
        let (id, seq) = {
            let state = lock_form(&f);
            let Some(id) = state.id.clone() else { return };
            (id, state.seq)
        };
        g.set_error("".into());
        g.set_notice("".into());
        g.set_busy(true);
        u.set_busy(true);
        let include_header_values = g.get_include_header_values();
        let (w2, f2) = (w.clone(), f.clone());
        spawn_bg(
            &b,
            move |bg| {
                bg.destination_request(DestinationReq::ExportSxcu {
                    id,
                    include_header_values,
                })
            },
            move |_, result| {
                let Some(app) = w2.upgrade() else { return };
                let u = app.global::<UploadsState>();
                let g = app.global::<DestinationForm>();
                g.set_busy(false);
                u.set_busy(false);
                if lock_form(&f2).seq != seq {
                    return;
                }
                match result {
                    Ok(DestinationResp::Exported { path: Some(path) }) => {
                        g.set_notice(format!("Exported to {path}").into())
                    }
                    Ok(DestinationResp::Exported { path: None }) => {}
                    Ok(other) => tracing::warn!("unexpected export response: {other:?}"),
                    Err(error) => g.set_error(format!("{error:#}").into()),
                }
            },
        );
    });

    let (w, f) = (weak.clone(), form.clone());
    g.on_add_header(move || {
        let Some(app) = w.upgrade() else { return };
        let mut state = lock_form(&f);
        state.headers.push(HeaderDraft::default());
        set_headers(&app.global::<DestinationForm>(), &state.headers);
    });

    let f = form.clone();
    g.on_header_name_edited(move |index, text| {
        if let Some(header) = lock_form(&f).headers.get_mut(index as usize) {
            header.name = text.to_string();
        }
    });

    let f = form.clone();
    g.on_header_value_edited(move |index, text| {
        if let Some(header) = lock_form(&f).headers.get_mut(index as usize) {
            header.value = text.to_string();
        }
    });

    if crate::dev_var("SNAPLINE_SEED_DESTINATIONS").is_ok() {
        seed(&backend);
        refresh(&weak, &backend);
    }
    if let Ok(id) = crate::dev_var("SNAPLINE_OPEN_DESTINATION") {
        match find(&backend, &id) {
            Some(view) => open(&weak, &form, &view),
            None => tracing::warn!("no destination with id {id}"),
        }
    }
    match crate::dev_var("SNAPLINE_FORM_ACTION").as_deref() {
        Ok("save") => g.invoke_save_destination(),
        Ok("test") => g.invoke_test_destination(),
        Ok("header") => g.invoke_add_header(),
        Ok("export") => g.invoke_export_sxcu(),
        _ => {}
    }
}

type RcBackend = std::rc::Rc<Backend>;

fn close_form(w: &Weak<SnaplineWindow>, form: &Shared) {
    {
        let mut state = lock_form(form);
        state.seq += 1;
        state.id = None;
        state.kind = None;
        state.imported_from_sxcu = false;
        state.headers.clear();
    }
    let Some(app) = w.upgrade() else { return };
    app.global::<UploadsState>().set_active_id("".into());
    let g = app.global::<DestinationForm>();
    g.set_confirm_delete(false);
    g.set_error("".into());
    g.set_notice("".into());
    g.set_test_state("idle".into());
    g.set_test_message("".into());
}

fn open(w: &Weak<SnaplineWindow>, form: &Shared, view: &DestinationViewDto) {
    let Some(app) = w.upgrade() else { return };
    fill(&app.global::<DestinationForm>(), &mut lock_form(form), view);
    let u = app.global::<UploadsState>();
    u.set_destination_error("".into());
    u.set_active_id(view.id.clone().into());
}

fn fill(g: &DestinationForm, state: &mut FormState, view: &DestinationViewDto) {
    state.id = Some(view.id.clone());
    state.kind = Some(view.kind);
    state.imported_from_sxcu = view.imported_from_sxcu;
    state.headers = view
        .headers
        .iter()
        .map(|header| HeaderDraft {
            name: header.name.clone(),
            value: String::new(),
            saved: header.saved,
        })
        .collect();

    let description = if view.kind == DestinationKindDto::Custom {
        CUSTOM_DESCRIPTION
    } else {
        ""
    };
    g.set_kind(kind_name(view.kind).into());
    g.set_title(title_for(view).into());
    g.set_description(description.into());
    g.set_error("".into());
    g.set_notice("".into());
    g.set_test_state("idle".into());
    g.set_test_message("".into());
    g.set_confirm_delete(false);
    g.set_include_header_values(false);

    g.set_secret("".into());
    let secret_saved = if view.kind == DestinationKindDto::Gist {
        view.configured
    } else {
        view.client_id_saved
    };
    g.set_secret_saved(secret_saved);
    g.set_acl_public_read(view.acl_public_read);

    g.set_endpoint(or_empty(&view.endpoint));
    g.set_region(or_empty(&view.region));
    g.set_bucket(or_empty(&view.bucket));
    g.set_path_prefix(or_empty(&view.path_prefix));
    g.set_access_key_id("".into());
    g.set_access_key_saved(view.access_key_id_saved);
    g.set_secret_access_key("".into());
    g.set_secret_access_key_saved(view.secret_access_key_saved);
    let pattern = match (&view.public_url_pattern, view.kind) {
        (Some(pattern), _) => pattern.clone(),
        (None, DestinationKindDto::S3) => S3_PUBLIC_URL_PATTERN.to_string(),
        (None, _) => String::new(),
    };
    g.set_public_url_pattern(pattern.into());

    g.set_host(or_empty(&view.host));
    g.set_port(view.port.unwrap_or(22).to_string().into());
    g.set_username(or_empty(&view.username));
    g.set_password_auth(view.auth_method.as_deref() != Some("private_key"));
    g.set_password("".into());
    g.set_password_saved(view.password_saved);
    g.set_private_key_path(or_empty(&view.private_key_path));
    g.set_passphrase("".into());
    g.set_passphrase_saved(view.passphrase_saved);
    g.set_remote_folder(or_empty(&view.remote_folder));

    g.set_name(view.name.as_str().into());
    g.set_method(view.method.as_deref().unwrap_or("POST").into());
    g.set_request_url(or_empty(&view.request_url));
    g.set_body(body_label(view.body.as_deref()).into());
    g.set_file_form_field(view.file_form_field.as_deref().unwrap_or("file").into());
    g.set_link(path_display(view.link.as_deref()).into());
    g.set_thumbnail(path_display(view.thumbnail.as_deref()).into());
    g.set_deletion(path_display(view.deletion.as_deref()).into());
    g.set_error_message(path_display(view.error_message.as_deref()).into());
    set_headers(g, &state.headers);
}

fn build_input(g: &DestinationForm, state: &FormState) -> Result<DestinationInput, String> {
    let kind = state.kind.ok_or("Choose a destination first.")?;
    let mut input = blank(kind, state.id.clone());
    match kind {
        DestinationKindDto::Gist => {
            let token = g.get_secret().trim().to_string();
            if token.is_empty() && !g.get_secret_saved() {
                return Err("Enter the GitHub token.".into());
            }
            input.client_id = non_empty(token);
            input.acl_public_read = Some(g.get_acl_public_read());
        }
        DestinationKindDto::S3 => {
            let region = g.get_region().trim().to_string();
            let bucket = g.get_bucket().trim().to_string();
            let access_key_id = g.get_access_key_id().trim().to_string();
            let secret_access_key = g.get_secret_access_key().trim().to_string();
            let pattern = g.get_public_url_pattern().trim().to_string();
            if region.is_empty() {
                return Err("Enter the S3 region.".into());
            }
            if bucket.is_empty() {
                return Err("Enter the S3 bucket.".into());
            }
            if access_key_id.is_empty() && !g.get_access_key_saved() {
                return Err("Enter the S3 access key ID.".into());
            }
            if secret_access_key.is_empty() && !g.get_secret_access_key_saved() {
                return Err("Enter the S3 secret access key.".into());
            }
            if !pattern.contains("<key>") {
                return Err(URL_PATTERN_ERROR.into());
            }
            input.endpoint = non_empty(g.get_endpoint().trim().to_string());
            input.region = Some(region);
            input.bucket = Some(bucket);
            input.path_prefix = Some(g.get_path_prefix().trim().to_string());
            input.access_key_id = non_empty(access_key_id);
            input.secret_access_key = non_empty(secret_access_key);
            input.public_url_pattern = Some(pattern);
            input.acl_public_read = Some(g.get_acl_public_read());
        }
        DestinationKindDto::Sftp => {
            let host = g.get_host().trim().to_string();
            let port_text = g.get_port().trim().to_string();
            let username = g.get_username().trim().to_string();
            let password_auth = g.get_password_auth();
            let key_path = g.get_private_key_path().trim().to_string();
            let password = g.get_password().to_string();
            let passphrase = g.get_passphrase().to_string();
            let pattern = g.get_public_url_pattern().trim().to_string();
            if host.is_empty() {
                return Err("Enter the SFTP host.".into());
            }
            let port = match port_text.parse::<u16>() {
                Ok(number) if number >= 1 => number,
                _ => return Err("The SFTP port must be between 1 and 65535.".into()),
            };
            if username.is_empty() {
                return Err("Enter the SFTP username.".into());
            }
            if !password_auth && key_path.is_empty() {
                return Err("Choose the SFTP private key file.".into());
            }
            if !pattern.contains("<key>") {
                return Err(URL_PATTERN_ERROR.into());
            }
            if password_auth && password.is_empty() && !g.get_password_saved() {
                return Err("Enter the SFTP password.".into());
            }
            let method = if password_auth {
                "password"
            } else {
                "private_key"
            };
            input.host = Some(host);
            input.port = Some(port);
            input.username = Some(username);
            input.auth_method = Some(method.to_string());
            input.private_key_path = if password_auth { None } else { Some(key_path) };
            input.password = if password_auth && !password.is_empty() {
                Some(password)
            } else {
                None
            };
            input.passphrase = if !password_auth && !passphrase.is_empty() {
                Some(passphrase)
            } else {
                None
            };
            input.remote_folder = Some(g.get_remote_folder().trim().to_string());
            input.public_url_pattern = Some(pattern);
        }
        DestinationKindDto::Custom => {
            let request_url = g.get_request_url().trim().to_string();
            let body = body_value(g.get_body().as_str()).to_string();
            let file_form_field = g.get_file_form_field().trim().to_string();
            let link = path_value(g.get_link().as_str());
            if request_url.is_empty() {
                return Err("Enter the request URL.".into());
            }
            if body == "MultipartFormData" && file_form_field.is_empty() {
                return Err("Enter the file form field.".into());
            }
            if link.is_empty() {
                return Err("Enter the link from the response.".into());
            }
            input.name = Some(g.get_name().trim().to_string());
            input.method = Some(g.get_method().to_string());
            input.request_url = Some(request_url);
            input.body = Some(body);
            input.file_form_field = Some(file_form_field);
            input.link = Some(link);
            input.thumbnail = non_empty(path_value(g.get_thumbnail().as_str()));
            input.deletion = non_empty(path_value(g.get_deletion().as_str()));
            input.error_message = non_empty(path_value(g.get_error_message().as_str()));
            input.headers = state
                .headers
                .iter()
                .map(|header| DestinationHeaderInput {
                    name: header.name.clone(),
                    value: (!header.value.is_empty()).then(|| header.value.clone()),
                })
                .collect();
            input.imported_from_sxcu = Some(state.imported_from_sxcu);
        }
    }
    Ok(input)
}

fn blank(kind: DestinationKindDto, id: Option<String>) -> DestinationInput {
    DestinationInput {
        id,
        kind,
        name: None,
        client_id: None,
        endpoint: None,
        region: None,
        bucket: None,
        access_key_id: None,
        secret_access_key: None,
        path_prefix: None,
        public_url_pattern: None,
        acl_public_read: None,
        method: None,
        request_url: None,
        body: None,
        file_form_field: None,
        link: None,
        thumbnail: None,
        deletion: None,
        error_message: None,
        headers: Vec::new(),
        imported_from_sxcu: None,
        host: None,
        port: None,
        username: None,
        auth_method: None,
        private_key_path: None,
        remote_folder: None,
        password: None,
        passphrase: None,
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn or_empty(value: &Option<String>) -> SharedString {
    value.as_deref().unwrap_or_default().into()
}

fn set_headers(g: &DestinationForm, headers: &[HeaderDraft]) {
    let rows: Vec<FormHeaderData> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| FormHeaderData {
            index: index as i32,
            name: header.name.as_str().into(),
            value: header.value.as_str().into(),
            saved: header.saved,
        })
        .collect();
    g.set_headers(ModelRc::new(VecModel::from(rows)));
}

fn kind_name(kind: DestinationKindDto) -> &'static str {
    match kind {
        DestinationKindDto::S3 => "s3",
        DestinationKindDto::Gist => "gist",
        DestinationKindDto::Sftp => "sftp",
        DestinationKindDto::Custom => "custom",
    }
}

fn title_for(view: &DestinationViewDto) -> String {
    match view.kind {
        DestinationKindDto::Custom if view.name == "Custom uploader" => {
            "Custom uploader".to_string()
        }
        DestinationKindDto::Custom => format!("Custom uploader · {}", view.name),
        _ => view.name.clone(),
    }
}

fn body_label(value: Option<&str>) -> &'static str {
    match value {
        Some("Binary") => "Binary",
        Some("FormURLEncoded") => "Form URL-encoded",
        Some("JSON") => "JSON",
        _ => "Multipart form data",
    }
}

fn body_value(label: &str) -> &'static str {
    match label {
        "Binary" => "Binary",
        "Form URL-encoded" => "FormURLEncoded",
        "JSON" => "JSON",
        _ => "MultipartFormData",
    }
}

fn path_display(value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return String::new();
    };
    if let Some(path) = value
        .strip_prefix("{json:")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        return if path.starts_with('$') {
            path.to_string()
        } else if path.starts_with('[') {
            format!("${path}")
        } else {
            format!("$.{path}")
        };
    }
    value.to_string()
}

fn path_value(display: &str) -> String {
    let value = display.trim();
    if value.is_empty() || value.starts_with('{') {
        return value.to_string();
    }
    if value.starts_with('$') {
        return format!("{{json:{value}}}");
    }
    value.to_string()
}

fn seed(backend: &Backend) {
    match backend.destinations() {
        Ok(list) if list.is_empty() => {}
        _ => return,
    }
    for kind in ["s3", "gist", "sftp", "custom"] {
        let result = destination_kind(kind)
            .and_then(|kind| backend.destination_request(DestinationReq::Add { kind }));
        if let Err(error) = result {
            tracing::warn!("seed failed: {error:#}");
        }
    }
}

fn find(backend: &Backend, id: &str) -> Option<DestinationViewDto> {
    backend
        .destinations()
        .ok()?
        .into_iter()
        .find(|view| view.id == id)
}

fn refresh(w: &Weak<SnaplineWindow>, backend: &Backend) {
    if let Err(error) = crate::reload(w, backend) {
        tracing::warn!("reload failed: {error:#}");
    }
}

fn report(w: &Weak<SnaplineWindow>, message: &str) {
    tracing::warn!("{message}");
    if let Some(app) = w.upgrade() {
        app.global::<UploadsState>()
            .set_destination_error(message.into());
    }
}

fn set_import_warning(w: &Weak<SnaplineWindow>, message: &str) {
    if let Some(app) = w.upgrade() {
        app.global::<UploadsState>()
            .set_import_warning(message.into());
    }
}
