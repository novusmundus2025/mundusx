use crate::{config::Config, ControlPlaneTokenHeader};
use std::io::{self, Write};

fn private_plane() -> Result<Option<String>, String> {
    let mut registry = crate::control_planes::Registry::load().map_err(|e| e.to_string())?;
    let mut options: Vec<(String, String)> = registry.control_planes.iter().map(|p| (p.name.clone(), p.url.clone())).collect();
    options.push(("Add new private control plane".into(), "Save it for future sessions".into()));
    let Some(index) = crate::select_menu_option(&["Saved private control planes".into()], &options, "↑↓ Select · Enter Continue · Esc Back", 0) else { return Ok(None); };
    if index < registry.control_planes.len() { return Ok(Some(registry.control_planes[index].url.clone())); }
    print!("Endpoint URL (Enter cancels): ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(|e| e.to_string())?;
    if input.trim().is_empty() { return Ok(None); }
    let url = crate::control_planes::normalize(&input).map_err(|e| e.to_string())?;
    print!("Profile name (Enter uses URL): ");
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut name = String::new();
    io::stdin().read_line(&mut name).map_err(|e| e.to_string())?;
    registry.add(if name.trim().is_empty() { &url } else { name.trim() }, &url).and_then(|_| registry.save()).map_err(|e| e.to_string())?;
    Ok(Some(url))
}

pub struct Changes {
    pub config: Config,
    pub model_changed: bool,
    pub plane_changed: bool,
    pub token: Option<(String, ControlPlaneTokenHeader)>,
}

fn token() -> Result<(String, ControlPlaneTokenHeader), String> {
    let value = crate::read_token_from_prompt("API key / token (hidden; Enter for none): ")?;
    let header = if value.is_empty() { ControlPlaneTokenHeader::Bearer } else {
        let choices = vec![("Bearer token".into(), "Standard API authentication".into()),
            ("Coder / EHDA token".into(), "Endpoint hosted behind Coder".into())];
        match crate::select_menu_option(&["Credential type".into()], &choices, "↑↓ Select · Enter Continue", 0) {
            Some(0) => ControlPlaneTokenHeader::Bearer,
            Some(1) => ControlPlaneTokenHeader::CoderSessionToken,
            _ => return Err("Credential edit cancelled; nothing was applied.".into()),
        }
    };
    Ok((value, header))
}

pub fn edit(original: &Config) -> Result<Option<Changes>, String> {
    let mut config = original.clone();
    let mut credentials = None;
    loop {
        let options = vec![
            ("Model".into(), config.active_model.clone().unwrap_or_else(|| "not selected".into())),
            ("Control plane".into(), config.control_plane_url.clone()),
            ("API key / token".into(), if credentials.is_some() { "Updated · hidden" } else { "Unchanged · hidden" }.into()),
            ("Apply changes and resume".into(), "Active work has finished; restart only if required".into()),
            ("Back — discard pending changes".into(), "Resume the existing settings".into()),
        ];
        match crate::select_menu_option(&["Contributor settings".into(), "No changes are applied until you choose Apply.".into()], &options,
                "↑↓ Select · Enter Edit · Esc Back", 0) {
            Some(0) => {
                let choices: Vec<(String, String)> = if let Some(cluster) = &config.contributed_cluster {
                    cluster.models.iter().map(|name| (name.clone(), "Model advertised by your external engine".into())).collect()
                } else {
                    let backend = crate::resolved_backend(&config);
                    crate::selectable_catalog_options_for(backend, crate::model_vram_budget_mb(&config, backend))
                        .iter().map(|model| (model.name.clone(), model.label.clone())).collect()
                };
                if choices.is_empty() { crate::theme::warn("No compatible model choices were found."); continue; }
                if let Some(index) = crate::select_menu_option(&["Choose model".into()], &choices, "↑↓ Select · Enter Continue · Esc Back", 0) {
                    config.active_model = Some(choices[index].0.clone());
                    if let Some(cluster) = &mut config.contributed_cluster { cluster.model = config.active_model.clone(); }
                }
            }
            Some(1) => {
                let choices = vec![("Public MundusX".into(), crate::PUBLIC_CONTROL_PLANE_URL.into()),
                    ("Private / saved control plane".into(), "Pick a saved endpoint or add a new one".into())];
                let url = match crate::select_menu_option(&["Choose control plane".into()], &choices, "↑↓ Select · Enter Continue · Esc Back", 0) {
                    Some(0) => crate::PUBLIC_CONTROL_PLANE_URL.into(),
                    Some(1) => match private_plane()? { Some(url) => url, None => continue },
                    _ => continue,
                };
                if url != config.control_plane_url {
                    // Collect credentials before changing the endpoint. Never
                    // carry the previous plane's API key into a new connection.
                    let new_credentials = token()?;
                    config.control_plane_url = url;
                    credentials = Some(new_credentials);
                }
            }
            Some(2) => credentials = Some(token()?),
            Some(3) => {
                let model_changed = config.active_model != original.active_model;
                let plane_changed = config.control_plane_url != original.control_plane_url;
                if !model_changed && !plane_changed && credentials.is_none() { return Ok(None); }
                return Ok(Some(Changes { config, model_changed, plane_changed, token: credentials }));
            }
            _ => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires interactive terminal"]
    fn terminal_smoke() {
        crate::theme::configure(crate::theme::ThemeSelection::Mundusx);
        let result = super::edit(&crate::config::Config::default());
        assert!(result.is_ok());
        println!("SETTINGS_CHANGED={}", result.unwrap().is_some());
    }
}
