//! Integration orchestration: reuse Skill installation and MCP configuration,
//! then report separate preparation, software and first-use checks.
use super::*;

impl Service {
    pub(crate) fn integration_service_probes(
        &self,
        workspace: &str,
    ) -> Result<Vec<crate::blender_service::Probe>> {
        if !["codex", "claude", "opencode"].contains(&workspace) {
            return Err("未知 AI 工作区".into());
        }
        let manifest = crate::integrations::catalog(&self.data)?;
        let catalog = crate::mcp::catalog(&self.data)?;
        let mut probes = Vec::new();
        for descriptor in manifest.integrations {
            let Some(mcp) = catalog.servers.iter().find(|server| {
                descriptor.related_mcp_id.as_deref() == Some(server.id.as_str())
                    && crate::blender_runtime::supported(server)
            }) else {
                continue;
            };
            let (revision, endpoint) = match crate::mcp::blender_service_endpoint(
                &self.mcp_context(),
                workspace,
                crate::mcp::config_key(mcp),
            ) {
                Ok((revision, endpoint)) => (Some(revision), endpoint),
                Err(error) => (None, Err(error)),
            };
            probes.push(crate::blender_service::Probe {
                id: descriptor.id,
                revision,
                endpoint,
            });
        }
        Ok(probes)
    }

    fn integration_mcp_descriptor(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<crate::mcp::Descriptor> {
        let view = crate::mcp::view(&self.data, &self.mcp_context(), workspace)?;
        let descriptor = view["catalog"]["servers"]
            .as_array()
            .and_then(|servers| servers.iter().find(|s| s["id"] == id))
            .ok_or("关联 MCP 目录尚未下载，请刷新接入目录后重试")?;
        serde_json::from_value(descriptor.clone()).map_err(|e| e.to_string())
    }

    fn integration_command_ready(descriptor: &crate::mcp::Descriptor) -> bool {
        let crate::mcp::Transport::Stdio { command, .. } = &descriptor.transport else {
            return true;
        };
        let path = Path::new(command);
        if path.is_absolute() {
            return path.is_file();
        }
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|directory| {
            directory.join(command).is_file()
                || cfg!(windows)
                    && ["exe", "cmd", "bat"]
                        .iter()
                        .any(|extension| directory.join(format!("{command}.{extension}")).is_file())
        })
    }

    fn prepare_integration_mcp(
        &mut self,
        workspace: &str,
        id: &str,
        expected: Option<&str>,
    ) -> Result<()> {
        if self.integration_mcp_descriptor(workspace, id).is_err() {
            let fetched = fetch_catalog_conditional(self.server_url(), &self.data, "/api/mcp")?;
            self.apply_mcp(fetched)?;
        }
        let mut descriptor = self.integration_mcp_descriptor(workspace, id)?;
        let context = self.mcp_context();
        let status = crate::mcp::descriptor_status(&self.data, &context, workspace, &descriptor)?;
        let revision = status["revision"].as_str().ok_or("MCP 配置版本缺失")?;
        if expected.is_some_and(|v| v != revision)
            || (expected.is_none() && status["installed"] == true)
        {
            return Err("MCP 配置已变化，请检查状态后重试接入".into());
        }
        if status["compatible"] != true || status["tokenAvailable"] != true {
            return Err("关联 MCP 不支持当前工作区，或缺少本机凭据，请检查 MCP 管理".into());
        }
        if status["canInstall"] != true && status["state"] != "adoptable" {
            return Err(
                "已有 MCP 配置发生冲突或被外部修改，已保留原配置，请在 MCP 管理中检查".into(),
            );
        }
        let raw = crate::mcp::catalog(&self.data)?
            .servers
            .into_iter()
            .find(|s| s.id == id);
        if raw.as_ref().is_some_and(crate::blender_runtime::supported) {
            crate::blender_runtime::prepare(&self.data)?;
            descriptor = self.integration_mcp_descriptor(workspace, id)?;
        } else if !Self::integration_command_ready(&descriptor) {
            return Err(
                "关联 MCP 的启动程序尚未安装，当前服务暂不支持自动准备该依赖；未写入不可运行的配置"
                    .into(),
            );
        }
        // The original revision is retained across downloads, so an external
        // edit made while preparing dependencies is never overwritten.
        crate::mcp::install_descriptor(&self.data, &context, workspace, &descriptor, revision)?;
        if raw.as_ref().is_some_and(crate::blender_runtime::supported) {
            crate::blender_runtime::prepare_addon(&self.data, &self.home, &self.app_data_dir())?;
        }
        Ok(())
    }

    fn prepare_integration_skill(
        &mut self,
        id: &str,
        workspace: &str,
        visiting: &mut HashSet<String>,
        adjacent: bool,
    ) -> Result<()> {
        if !visiting.insert(id.into()) {
            return Err(format!("共享依赖存在循环引用：{id}"));
        }
        let skill = self.skill(id)?;
        let path = self.target_path(&skill, workspace, "global", "")?;
        let existing = if adjacent {
            self.discover_installations(None)
                .into_iter()
                .find(|entry| {
                    entry.exists
                        && entry.record.scope == "global"
                        && crate::installed::matches(&entry.record, &skill)
                        && Path::new(&entry.record.path)
                            .parent()
                            .map(crate::installed::path_key)
                            == path.parent().map(crate::installed::path_key)
                })
                .map(|entry| entry.record)
        } else {
            self.existing_skill(&skill, workspace, "global", &path)
        };
        if existing.is_none() {
            self.download(id)?;
            let mut chain = visiting
                .iter()
                .filter(|ancestor| ancestor.as_str() != id)
                .cloned()
                .collect();
            self.install_skill(id, workspace, "global", "", &mut chain)?;
        } else if let Some(record) = existing {
            if !self.integration_dependencies_ready(&skill, &record, &mut HashSet::new()) {
                if !Self::owns(&record) || record.target != workspace {
                    return Err("已有 Skill 的运行依赖不完整，且不属于当前工作区管理；已保留原文件，请在原工作区检查".into());
                }
                for dependency in &skill.shared_dependencies {
                    self.prepare_integration_skill(dependency, workspace, visiting, true)?;
                }
                if !self.integration_dependencies_ready(&skill, &record, &mut HashSet::new()) {
                    self.download(id)?;
                    self.reinstall_skill(id, workspace, "global", "", true)?;
                }
            }
        }
        visiting.remove(id);
        Ok(())
    }

    fn integration_dependencies_ready(
        &self,
        skill: &Skill,
        record: &Record,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if !visiting.insert(skill.id.clone()) {
            return false;
        }
        let ready = skill.requirements.iter().all(|requirement| {
            record.runtimes.iter().any(|runtime| {
                crate::runtime::compatible(runtime, &requirement.kind, &requirement.minimum)
                    && Path::new(&runtime.path).is_file()
            })
        }) && record
            .runtimes
            .iter()
            .all(|runtime| Path::new(&runtime.path).is_file())
            && (skill.python_dependencies.is_empty()
                || Path::new(&record.path).join(".cyapi").is_dir())
            && crate::external_tools::cached(&self.data, &skill.tools)
            && skill.shared_dependencies.iter().all(|id| {
                let Ok(dependency) = self.skill(id) else {
                    return false;
                };
                // Shared resources are referenced relatively, so another workspace's
                // installation cannot satisfy a missing adjacent dependency.
                Path::new(&record.path).parent().is_some_and(|parent| {
                    crate::installed::scan(parent, &record.target, &record.scope)
                        .iter()
                        .find(|candidate| crate::installed::matches(candidate, &dependency))
                        .is_some_and(|candidate| {
                            let installed = self
                                .records
                                .iter()
                                .find(|managed| {
                                    crate::installed::path_key(Path::new(&managed.path))
                                        == crate::installed::path_key(Path::new(&candidate.path))
                                        && Self::owns(managed)
                                })
                                .unwrap_or(candidate);
                            self.integration_dependencies_ready(&dependency, installed, visiting)
                        })
                })
            });
        visiting.remove(&skill.id);
        ready
    }

    fn integration_requirement_satisfied(
        &self,
        requirement: &crate::integrations::Requirement,
        workspace: &str,
    ) -> bool {
        match requirement.kind.as_str() {
            "skill" => self.skill(&requirement.id).ok().is_some_and(|skill| {
                self.target_path(&skill, workspace, "global", "")
                    .ok()
                    .is_some_and(|path| {
                        self.existing_skill(&skill, workspace, "global", &path)
                            .is_some()
                    })
            }),
            "tool" => self.catalog.as_ref().is_some_and(|catalog| {
                catalog
                    .skills
                    .iter()
                    .flat_map(|skill| &skill.tools)
                    .any(|tool| {
                        tool.name == requirement.id
                            && tool.platform == crate::platform::current()
                            && crate::external_tools::cached(&self.data, std::slice::from_ref(tool))
                    })
            }),
            _ => false,
        }
    }

    fn integration_missing(
        &self,
        descriptor: &crate::integrations::Descriptor,
        workspace: &str,
    ) -> Vec<String> {
        descriptor
            .requires
            .iter()
            .filter(|r| !r.optional)
            .filter(|r| !self.integration_requirement_satisfied(r, workspace))
            .map(|r| r.id.clone())
            .collect()
    }

    fn integration_skill_view(
        &self,
        descriptor: &crate::integrations::Descriptor,
        workspace: &str,
    ) -> Result<Value> {
        let id = descriptor.skill_id.as_deref().ok_or("Skill 声明缺失")?;
        let Ok(skill) = self.skill(id) else {
            return Ok(
                json!({"id":descriptor.id,"downloaded":false,"registered":false,"managed":false,
                "updateAvailable":false,"usable":false,"health":{"ready":false,"message":"请先同步 Skill 目录","rows":[]}}),
            );
        };
        let path = self.target_path(&skill, workspace, "global", "")?;
        let existing = self.existing_skill(&skill, workspace, "global", &path);
        let installed = existing.is_some();
        let owned = existing.as_ref().is_some_and(Self::owns);
        let current = existing
            .as_ref()
            .is_some_and(|record| record.revision == skill.revision);
        // Cache readiness describes install resources; it is not a runtime health check.
        let downloaded = self.ready(&skill);
        let dependencies_ready = existing.as_ref().is_some_and(|record| {
            self.integration_dependencies_ready(&skill, record, &mut HashSet::new())
        });
        let mut rows = vec![
            json!({"label":"Skill","value":id}),
            json!({"label":"技能版本","value":skill.version}),
        ];
        if let Some(record) = &existing {
            rows.push(json!({"label":"安装位置","value":record.path}));
        }
        Ok(
            json!({"id":descriptor.id,"downloaded":downloaded,"registered":installed,"managed":owned && existing.as_ref().is_some_and(|r| r.target == workspace),
            "updateAvailable":installed && owned && !current,"usable":installed && dependencies_ready,
            "health":{"ready":installed && dependencies_ready,"message":if !installed {"尚未安装技能"} else if !dependencies_ready {"运行依赖需要检查"} else {"技能已安装，实际任务待验证"},"rows":rows}}),
        )
    }

    pub(super) fn integrations_view(&self, workspace: &str) -> Result<Value> {
        let manifest = crate::integrations::catalog(&self.data)?;
        let mut view = crate::integrations::view(&self.data, &self.mcp_context(), workspace)?;
        let software = crate::software::detect(&manifest.integrations);
        let mcp_view = if manifest
            .integrations
            .iter()
            .any(|d| d.related_mcp_id.is_some())
        {
            Some(crate::mcp::view(
                &self.data,
                &self.mcp_context(),
                workspace,
            )?)
        } else {
            None
        };
        for descriptor in &manifest.integrations {
            let mut entry = if descriptor.skill_id.is_some() {
                self.integration_skill_view(descriptor, workspace)?
            } else {
                view["entries"]
                    .as_array()
                    .and_then(|entries| entries.iter().find(|e| e["id"] == descriptor.id))
                    .cloned()
                    .ok_or("接入状态缺失")?
            };
            let compatible_workspace = descriptor.workspaces.is_empty()
                || descriptor.workspaces.iter().any(|w| w == workspace);
            let missing = self.integration_missing(descriptor, workspace);
            entry["requirementsMet"] = json!(missing.is_empty());
            entry["compatible"] = json!(compatible_workspace);
            entry["requires"] = json!(descriptor
                .requires
                .iter()
                .map(|r| json!({"kind":r.kind,"id":r.id,"optional":r.optional,
                "satisfied":self.integration_requirement_satisfied(r, workspace)}))
                .collect::<Vec<_>>());
            let detected = software
                .get(&descriptor.id)
                .cloned()
                .unwrap_or_else(|| json!({"found":null,"version":null}));
            let assessment = crate::software::assess(
                descriptor.compatibility.as_ref(),
                detected["version"].as_str(),
            );
            entry["software"] = json!({"found":detected["found"],"version":detected["version"],"source":detected["source"],
                "state":assessment["state"],"message":assessment["message"]});
            let mut checks = vec![json!({"id":"version","title":"软件版本",
                "state":match assessment["state"].as_str() {Some("incompatible")=>"blocked",Some("tested"|"untested")=>"ready",_=>"unverified"},
                "message":assessment["message"]})];
            checks.push(json!({"id":"capability","title":if descriptor.skill_id.is_some() {"技能与运行依赖"} else {"接入组件与配置"},
                "state":if entry["usable"] == true {"ready"} else {"blocked"},
                "message":entry["health"]["message"]}));
            if !compatible_workspace || !missing.is_empty() {
                checks.push(json!({"id":"prerequisites","title":"接入条件","state":"blocked",
                    "message":if !compatible_workspace {"此接入尚不支持当前 AI 工具".into()} else {format!("缺少前置条件：{}",missing.join("、"))}}));
            }
            if let Some(id) = &descriptor.related_mcp_id {
                let status = mcp_view
                    .as_ref()
                    .and_then(|v| v["entries"].as_array())
                    .and_then(|entries| entries.iter().find(|e| e["id"] == *id));
                let configured =
                    status.is_some_and(|s| s["recognized"] == true && s["enabled"] == true);
                let blender = crate::mcp::catalog(&self.data).ok().is_some_and(|catalog| {
                    catalog
                        .servers
                        .iter()
                        .any(|s| s.id == *id && crate::blender_runtime::supported(s))
                });
                let runtime = if blender {
                    Some(crate::blender_runtime::status(
                        &self.data,
                        &self.home,
                        &self.app_data_dir(),
                    ))
                } else {
                    None
                };
                let runtime_ready = if let Some(runtime) = &runtime {
                    runtime["runtimeReady"] == true
                } else {
                    self.integration_mcp_descriptor(workspace, id)
                        .is_ok_and(|d| Self::integration_command_ready(&d))
                };
                entry["relatedMcp"] = json!({"id":id,"configured":configured,"runtimeReady":runtime_ready,"state":status.map(|s|&s["state"])});
                entry["mcpRevision"] = status.map(|s| s["revision"].clone()).unwrap_or(Value::Null);
                checks.push(json!({"id":"mcp-runtime","title":"MCP 运行环境","state":if runtime_ready {"ready"} else {"blocked"},
                    "message":if runtime_ready {"启动程序已准备"} else {"运行环境尚未准备；点击一键接入将自动准备支持的依赖"}}));
                checks.push(json!({"id":"connection","title":"MCP 配置","state":if configured {"ready"} else {"blocked"},
                    "message":if configured {"当前 AI 工具的 MCP 配置已写入并启用；软件内服务与实际调用仍需验证"} else {"尚未配置并启用；点击一键接入会自动处理，无需另行安装 MCP"}}));
                if let Some(runtime) = &runtime {
                    entry["addonReady"] = runtime["addonReady"].clone();
                    checks.push(json!({"id":"addon-files","title":"Blender 插件文件","state":if runtime["addonReady"] == true {"ready"} else {"unverified"},"message":runtime["addonMessage"]}));
                    entry["service"] = json!(crate::blender_service::Status::unverified("正在检测 Blender 服务；启用插件后还需在 3D 视图按 N，打开 MCP for Blender，点击 Connect to MCP server"));
                    checks.push(json!({"id":"software-service","title":"Blender 服务连接","state":"unverified","message":entry["service"]["message"]}));
                } else {
                    checks.push(json!({"id":"software-service","title":"软件内服务","state":"unverified","message":"请按教程启动软件内服务；配置完成不代表实际调用成功"}));
                }
                if !configured || !runtime_ready {
                    entry["usable"] = json!(false);
                }
            } else if descriptor.skill_id.is_none() {
                checks.push(json!({"id":"connection","title":"本机软件连接",
                    "state":if entry["health"]["ready"] == true {"ready"} else {"blocked"},"message":entry["health"]["message"]}));
            } else {
                checks.push(
                    json!({"id":"connection","title":"软件内检查","state":"unverified",
                    "message":"请按教程在目标软件中打开并检查生成结果"}),
                );
            }
            checks.push(json!({"id":"first-use","title":"首次任务验证","state":"unverified",
                "message":descriptor.guide.as_ref().map(|g|g.outcome.as_str()).unwrap_or("重新打开 AI 会话并验证一次实际任务")}));
            let blocked = checks.iter().any(|c| c["state"] == "blocked");
            let state = if entry["registered"] != true {
                "not-installed"
            } else if blocked {
                "needs-attention"
            } else if descriptor.skill_id.is_none() {
                "ready"
            } else {
                "configured"
            };
            if !missing.is_empty() || !compatible_workspace || assessment["state"] == "incompatible"
            {
                entry["usable"] = json!(false);
            }
            entry["readiness"] = json!({"state":state,"checks":checks});
            if let Some(entries) = view["entries"].as_array_mut() {
                if let Some(previous) = entries.iter_mut().find(|e| e["id"] == descriptor.id) {
                    *previous = entry;
                } else {
                    entries.push(entry);
                }
            }
        }
        Ok(view)
    }

    pub(super) fn integration_request(&mut self, request: &Request) -> Result<Value> {
        // Tauri prefetches catalog refreshes outside the service lock. Local actions
        // reuse the cached catalog and existing installers.
        let workspace = request.workspace.as_deref().unwrap_or("codex");
        if !["codex", "claude", "opencode"].contains(&workspace) {
            return Err("未知 AI 工作区".into());
        }
        if request.action == "integration-open-download" {
            return crate::integrations::open_download(
                &self.data,
                request.integration_id.as_deref().ok_or("缺少接入标识")?,
            );
        }
        if request.action.starts_with("integration-") {
            let id = request.integration_id.as_deref().ok_or("缺少接入标识")?;
            let descriptor = crate::integrations::descriptor(&self.data, id)?;
            if request.action != "integration-disconnect" {
                if !descriptor.workspaces.is_empty()
                    && !descriptor.workspaces.iter().any(|w| w == workspace)
                {
                    return Err("此接入不支持当前 AI 工具".into());
                }
                let missing = self.integration_missing(&descriptor, workspace);
                if !missing.is_empty() {
                    return Err(format!("缺少前置条件：{}", missing.join("、")));
                }
            }
            if let Some(skill_id) = &descriptor.skill_id {
                match request.action.as_str() {
                    "integration-connect" => {
                        if self.skill(skill_id).is_err() {
                            self.refresh()?;
                        }
                        self.prepare_integration_skill(
                            skill_id,
                            workspace,
                            &mut HashSet::new(),
                            false,
                        )?;
                        if let Some(id) = &descriptor.related_mcp_id {
                            self.prepare_integration_mcp(workspace, id, request.configuration_revision.as_deref())
                                .map_err(|error| format!("Skill 已安装，但接入尚未完成：{error}。已完成的步骤会保留，可重试补全接入。"))?;
                        }
                        if workspace == "codex" {
                            crate::integrations::retire_previous(
                                &self.data,
                                &self.codex_home(),
                                &descriptor,
                            )?;
                        }
                    }
                    "integration-disconnect" => {
                        let skill = self.skill(skill_id)?;
                        let path = self.target_path(&skill, workspace, "global", "")?;
                        if let Some(record) =
                            self.existing_skill(&skill, workspace, "global", &path)
                        {
                            if record.target != workspace {
                                return Err("此 Skill 从兼容目录加载，请在原安装工作区管理".into());
                            }
                            self.remove(&record.id)?;
                        }
                    }
                    _ => return Err("此接入通过 Skill 执行任务，请在对应 AI 工具中使用".into()),
                }
            } else if request.action == "integration-action" {
                return crate::integrations::action(
                    &self.data,
                    &descriptor,
                    request.action_id.as_deref().ok_or("缺少操作标识")?,
                    request.values.as_ref().unwrap_or(&Default::default()),
                );
            } else {
                let revision = request
                    .configuration_revision
                    .as_deref()
                    .ok_or("缺少配置版本，请重新检测")?;
                match request.action.as_str() {
                    "integration-connect" => {
                        if !crate::integrations::installed(&self.data, &descriptor) {
                            let component = crate::integrations::platform_component(&descriptor)
                                .ok_or("服务端未提供当前系统的接入组件")?;
                            let bytes = self.fetch(&component.path, component.size, 120)?;
                            crate::integrations::install(&self.data, &descriptor, &bytes)?;
                        }
                        self.retire_components();
                        crate::integrations::connect(
                            &self.data,
                            &self.mcp_context(),
                            workspace,
                            &descriptor,
                            revision,
                        )?;
                    }
                    "integration-adopt" => crate::integrations::adopt(
                        &self.data,
                        &self.mcp_context(),
                        workspace,
                        &descriptor,
                        revision,
                    )?,
                    "integration-disconnect" => crate::integrations::disconnect(
                        &self.data,
                        &self.mcp_context(),
                        workspace,
                        &descriptor,
                        revision,
                    )?,
                    _ => return Err("未知接入操作".into()),
                }
            }
        }
        self.integrations_view(workspace)
    }
}
