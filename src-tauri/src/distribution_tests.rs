use super::*;
use std::process::Command;

// This runs the production downloader, ABI selection, offline venv installer,
// launchers, conflict checks and uninstaller against every published Skill.
// The wrapper starts a loopback HTTP server. All user directories are isolated.
#[test]
#[ignore = "run npm run verify:skills in server/ with Python 3.12 and 3.14"]
fn published_skills_install_offline_in_all_agents() {
    let root = env::var_os("CYAPI_TEST_RESUME_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::temp_dir().join(format!("cyapi-all-skills-{}-{}", std::process::id(), now()))
        });
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("cyapi-all-skills-"));
    assert_eq!(
        runtime::normalize_path(root.parent().unwrap()),
        runtime::normalize_path(&env::temp_dir())
    );
    println!("Isolated audit directory: {}", root.display());
    fs::create_dir_all(root.join("project")).unwrap();
    let mut service = Service::new(root.join("data"), root.join("home")).unwrap();
    service.settings.server_url = env::var("CYAPI_TEST_SERVER").expect("Use verify:skills");
    service.refresh().unwrap();
    let mut skills = service.catalog.as_ref().unwrap().skills.clone();
    if let Ok(selection) = env::var("CYAPI_TEST_SKILLS") {
        let selected: HashSet<_> = selection.split(',').collect();
        skills.retain(|skill| selected.contains(skill.id.as_str()));
        assert_eq!(skills.len(), selected.len(), "Unknown selected Skill");
    }
    for skill in &skills {
        if skill.distribution == "source-only" {
            assert!(service.download(&skill.id).is_err());
            assert!(service.install(&skill.id, "codex", "global", "").is_err());
        } else {
            println!("Download {}", skill.id);
            service
                .download(&skill.id)
                .unwrap_or_else(|error| panic!("{}: {error}", skill.id));
            assert!(
                service.ready(skill),
                "{} is not ready after download",
                skill.id
            );
        }
    }
    // No installation below can fall back to fetching an omitted dependency.
    service.settings.server_url = "http://127.0.0.1:1".into();
    let project = root.join("project").to_string_lossy().into_owned();
    let mut installed = 0;
    let mut removed = 0;
    let mut environments = HashSet::new();
    for target in ["codex", "claude", "opencode"] {
        for scope in ["global", "project"] {
            for skill in skills
                .iter()
                .filter(|skill| skill.distribution != "source-only")
            {
                let destination = service.target_path(skill, target, scope, &project).unwrap();
                // A shared support package may already have been installed by
                // its first dependent Skill earlier in this same scope.
                let existing = service
                    .records
                    .iter()
                    .find(|record| {
                        record.skill_id == skill.id
                            && record.target == target
                            && record.scope == scope
                    })
                    .cloned();
                let record = match existing {
                    Some(record) if record.package_sha256 == skill.sha256 => record,
                    previous => {
                        if let Some(record) = previous {
                            service.remove(&record.id).unwrap();
                        }
                        service
                            .install(&skill.id, target, scope, &project)
                            .unwrap_or_else(|error| {
                                panic!("{target}/{scope}/{}: {error}", skill.id)
                            })
                    }
                };
                assert_eq!(Path::new(&record.path), destination);
                assert!(destination.join("SKILL.md").is_file());
                assert!(destination.join("upstream-notices/SOURCE.json").is_file());
                for dependency in &skill.shared_dependencies {
                    assert!(destination
                        .parent()
                        .unwrap()
                        .join(dependency)
                        .join("SKILL.md")
                        .is_file());
                }
                if installed
                    % skills
                        .iter()
                        .filter(|item| item.distribution != "source-only")
                        .count()
                    == 0
                {
                    assert!(
                        service.install(&skill.id, target, scope, &project).is_err(),
                        "Duplicate installation must be refused"
                    );
                }
                for runtime in &record.runtimes {
                    let kind = &runtime.kind;
                    let launcher = destination.join(format!(".cyapi/run-{kind}.ps1"));
                    assert!(launcher.is_file());
                    if environments.insert(runtime.path.clone()) {
                        let output = Command::new("powershell.exe")
                            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                            .arg(&launcher)
                            .arg("--version")
                            .output()
                            .unwrap();
                        assert!(
                            output.status.success(),
                            "Launcher failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        if runtime.source == "shared-venv" {
                            let imports = match skill.id.as_str() {
                                "jianying-editor" => {
                                    "import cv2,imageio,numpy,requests,psutil,websockets"
                                }
                                "nature-academic-search" => {
                                    "import mcp,requests,toml,lxml,pybliometrics,defusedxml; from lxml import etree"
                                }
                                "nature-downloader" => "import pdfplumber,pypdf,yaml,jsonschema",
                                "nature-figure" => r#"
import io, pymupdf, numpy, matplotlib, scipy, seaborn
matplotlib.use('Agg')
from matplotlib import pyplot as plt
from PIL import Image
values = numpy.array([2., 4., 6.])
figure, axes = plt.subplots()
axes.scatter([0, 0, 0], values)
axes.errorbar([0], [values.mean()], yerr=[values.std(ddof=1)], fmt='o')
png, pdf = io.BytesIO(), io.BytesIO()
figure.savefig(png, format='png')
figure.savefig(pdf, format='pdf')
plt.close(figure)
png.seek(0)
Image.open(png).verify()
with pymupdf.open(stream=pdf.getvalue(), filetype='pdf') as document:
    assert len(document) == 1
    assert document[0].get_pixmap().width > 0
"#,
                                "nature-paper2ppt" => r#"
import io, pymupdf
from PIL import Image
from pptx import Presentation
from pptx.chart.data import CategoryChartData
from pptx.enum.chart import XL_CHART_TYPE
from pptx.util import Inches
with pymupdf.open() as document:
    page = document.new_page()
    page.insert_text((72, 72), 'Synthetic dependency smoke test')
    pdf = document.tobytes()
with pymupdf.open(stream=pdf, filetype='pdf') as document:
    assert 'Synthetic dependency smoke test' in document[0].get_text()
    thumbnail = io.BytesIO(document[0].get_pixmap().tobytes('png'))
Image.open(thumbnail).verify()
thumbnail.seek(0)
deck = Presentation()
slide = deck.slides.add_slide(deck.slide_layouts[6])
slide.shapes.add_picture(thumbnail, 0, 0, height=Inches(2))
data = CategoryChartData()
data.categories = ['A', 'B', 'C']
data.add_series('Response', [4, 8, 12])
slide.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(2), 0, Inches(6), Inches(4), data)
slide.notes_slide.notes_text_frame.text = 'Source: synthetic smoke test'
output = io.BytesIO()
deck.save(output)
output.seek(0)
reopened = Presentation(output)
assert len(reopened.slides) == 1
assert 'synthetic' in reopened.slides[0].notes_slide.notes_text_frame.text
chart = next(shape.chart for shape in reopened.slides[0].shapes if shape.has_chart)
assert tuple(chart.series[0].values) == (4, 8, 12)
"#,
                                "nature-paper-card" => "import fitz",
                                "nature-citation" => "import defusedxml.ElementTree",
                                "nature-proposal-writer" => "import docx",
                                "nature-image2ppt" => {
                                    "import pypdfium2,PIL,numpy,requests,yaml,openai,defusedxml"
                                }
                                "nature-paper-to-patent" => {
                                    "import docx,pptx,matplotlib,mammoth,latex2mathml,pypdf; from playwright.sync_api import sync_playwright"
                                }
                                "nature-paper-trans" => "import pymupdf; from PIL import Image",
                                _ => panic!("Add runtime import check for {}", skill.id),
                            };
                            let output = Command::new(&runtime.path)
                                .args(["-I", "-c", imports])
                                .output()
                                .unwrap();
                            assert!(
                                output.status.success(),
                                "{} imports failed: {}",
                                skill.id,
                                String::from_utf8_lossy(&output.stderr)
                            );
                            if skill.id == "nature-academic-search" {
                                let smoke = r#"
import asyncio, json, sys
sys.path.insert(0, sys.argv[1])
import academic_search_server as server
names = {tool.name for tool in asyncio.run(server.mcp.list_tools())}
assert {'search_papers', 'get_paper_by_id'}.issubset(names), names
print(json.dumps(sorted(names)))
"#;
                                let output = Command::new(&runtime.path)
                                    .args(["-I", "-c", smoke])
                                    .arg(destination.join("mcp-server"))
                                    .output()
                                    .unwrap();
                                assert!(output.status.success(), "Academic MCP startup failed: {}",
                                    String::from_utf8_lossy(&output.stderr));
                                println!("Academic MCP tools: {}", String::from_utf8_lossy(&output.stdout));
                            }
                        }
                    }
                }
                installed += 1;
            }
            println!("Verified {target}/{scope}; cumulative installations: {installed}");
            // Remove each completed scope before testing the next Agent.
            // OpenCode imports Claude and .agents Skills, so leaving those
            // fixtures installed correctly blocks its explicit installation.
            let records: Vec<_> = service
                .records
                .iter()
                .filter(|record| record.target == target && record.scope == scope)
                .cloned()
                .collect();
            assert_eq!(
                records.len(),
                skills
                    .iter()
                    .filter(|skill| skill.distribution != "source-only")
                    .count()
            );
            for record in records.iter().rev() {
                service.remove(&record.id).unwrap();
                assert!(!Path::new(&record.path).exists());
                assert!(service
                    .data
                    .join("backups")
                    .join(&record.id)
                    .join("SKILL.md")
                    .is_file());
                removed += 1;
            }
        }
    }
    assert_eq!(removed, installed);
    assert!(service.records.is_empty());
    for executable in &environments {
        assert!(
            Path::new(executable).is_file(),
            "Uninstall removed shared runtime"
        );
    }
    println!(
        "Verified {installed} offline installations and removals; {} reusable runtimes",
        environments.len()
    );
    fs::remove_dir_all(root).unwrap();
}
