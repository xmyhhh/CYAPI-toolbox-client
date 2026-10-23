//! Integration orchestration: reuse Skill installation and MCP configuration,
//! then report separate preparation, software and first-use checks.
use super::*;

impl Service {
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
                entry["relatedMcp"] =
                    json!({"id":id,"configured":configured,"state":status.map(|s|&s["state"])});
                checks.push(json!({"id":"connection","title":"MCP 工具连接","state":if configured {"unverified"} else {"blocked"},
                    "message":if configured {"连接已配置并启用；软件内服务与实际调用仍需验证"} else {"请前往 MCP 管理配置并启用对应连接"}}));
                if !configured {
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
                        let skill = self.skill(skill_id)?;
                        let path = self.target_path(&skill, workspace, "global", "")?;
                        if self
                            .existing_skill(&skill, workspace, "global", &path)
                            .is_none()
                        {
                            self.download(skill_id)?;
                            self.install(skill_id, workspace, "global", "")?;
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
