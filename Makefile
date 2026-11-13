# Common development commands for spinorama.

.DEFAULT_GOAL := help

PYTHON ?= $(CURDIR)/.venv/bin/python3
MATURIN ?= $(CURDIR)/.venv/bin/maturin
CARGO ?= cargo
NPM ?= npm
WASM_PACK ?= wasm-pack
GPUI_TOOLKIT_DIR ?= $(abspath $(CURDIR)/../../gpui-toolkit)
RENDER_TARGET_DIR ?= $(GPUI_TOOLKIT_DIR)/target
RENDER_IMAGES_BIN ?= $(RENDER_TARGET_DIR)/release/spinorama-render-images
GENERATE_GRAPH_BIN ?= $(abspath $(or $(CARGO_TARGET_DIR),$(CURDIR)/target))/release/generate-graph
METADATA_DIR ?= datas/measurements
MEASUREMENTS_ROOT ?= datas/measurements
JOBS ?= 1
VERSION ?=
DEPLOY_DEV_URL ?= https://dev.spinorama.org

ANNOTATIONS_MANIFEST := src/spinorama/annotations_rust/Cargo.toml
GENERATE_MANIFEST := crates/spinorama-generate/Cargo.toml
WORKSPACE_MANIFEST := Cargo.toml
ANNOTATIONS_CRATE := src/spinorama/annotations_rust
WASM_BUILD_DIR := $(CURDIR)/build/website/annotations-rust
WASM_DIST_DIR := $(CURDIR)/dist/js/annotations-rust

DEV_HOST ?= 127.0.0.1
DEV_PORT ?= 8888
DEV_URL ?= http://localhost:$(DEV_PORT)
SPEAKER ?=
export PYTHONPATH := .:./src:./src/spinorama:./src/website:./scripts
PYTHON_LINT_PATHS ?= src scripts tests

.PHONY: help setup install-js rust rust-build rust-python rust-check rust-fmt \
	rust-clippy wasm build build-dev build-dev-full build-dev-site rebuild serve dev run \
	test test-rust test-python test-js test-dev test-annotations test-annotations-rust lint lint-python \
	lint-python-strict check annotations clean generate-check generate-fmt generate-clippy \
	generate-test generate-build renderer-build graphs images check-meta check-meta-files check-public-dist \
	workspace-check workspace-fmt workspace-clippy workspace-test workspace-build \
	api-check api-fmt api-clippy api-test api-build deploy-dev deploy-nginx-dev \
	deploy-binaries-dev deploy-binaries-prod deploy-binaries-all stage-api-prod

GRAPH_SELECTION = $(if $(strip $(SPEAKER)),--speaker "$(SPEAKER)") $(if $(strip $(VERSION)),--mversion "$(VERSION)")

help: ## Show available targets and configurable variables.
	@awk 'BEGIN {FS = ":.*## "; printf "Usage: make <target> [VARIABLE=value]\n\nTargets:\n"} /^[a-zA-Z0-9_.-]+:.*## / {printf "  %-18s %s\n", $$1, $$2} END {printf "\nExamples:\n  make dev\n  make images SPEAKER=\"Topping MA4\"\n  make test-annotations\n"}' $(MAKEFILE_LIST)
	@printf '\nImage generation: SPEAKER and VERSION filter the catalogue; omit them for all measurements.\n  GPUI_TOOLKIT_DIR=%s\n  JOBS=%s\nDeployment: make deploy-dev | deploy-nginx-dev | deploy-binaries-dev\n' "$(GPUI_TOOLKIT_DIR)" "$(JOBS)"

setup: ## Install project dependencies and compile native extensions.
	./scripts/setup.sh

install-js: ## Install JavaScript dependencies from the lockfile.
	$(NPM) ci

rust: workspace-build ## Build the release Rust workspace binaries.

rust-build: ## Build the optional Python annotation wheel with Maturin.
	$(MATURIN) build --release --manifest-path $(ANNOTATIONS_MANIFEST)

rust-python: ## Build and install the optional Python annotation extension with Maturin.
	$(MATURIN) develop --release --manifest-path $(ANNOTATIONS_MANIFEST)

rust-check: ## Type-check the native Rust annotation solver.
	$(CARGO) check --manifest-path $(ANNOTATIONS_MANIFEST)

rust-fmt: ## Check Rust formatting without modifying files.
	$(CARGO) fmt --manifest-path $(ANNOTATIONS_MANIFEST) -- --check

rust-clippy: ## Lint the Rust annotation solver and deny warnings.
	$(CARGO) clippy --manifest-path $(ANNOTATIONS_MANIFEST) -- -D warnings

generate-check: ## Type-check the Rust graph generator crate.
	$(CARGO) check --manifest-path $(GENERATE_MANIFEST)

generate-fmt: ## Check Rust graph generator formatting without modifying files.
	$(CARGO) fmt --manifest-path $(GENERATE_MANIFEST) -- --check

generate-clippy: ## Lint the Rust graph generator and deny warnings.
	$(CARGO) clippy --manifest-path $(GENERATE_MANIFEST) --all-targets -- -D warnings

generate-test: ## Run Rust graph generator tests.
	$(CARGO) test --manifest-path $(GENERATE_MANIFEST)

workspace-check: ## Type-check the whole Rust workspace.
	$(CARGO) check --manifest-path $(WORKSPACE_MANIFEST) --workspace

workspace-fmt: ## Check formatting of the whole Rust workspace.
	$(CARGO) fmt --manifest-path $(WORKSPACE_MANIFEST) --all -- --check

workspace-clippy: ## Lint the whole Rust workspace and deny warnings.
	$(CARGO) clippy --manifest-path $(WORKSPACE_MANIFEST) --workspace --all-targets -- -D warnings

workspace-test: ## Run all Rust workspace tests.
	$(CARGO) test --manifest-path $(WORKSPACE_MANIFEST) --workspace

workspace-build: ## Build all release binaries of the Rust workspace.
	$(CARGO) build --release --manifest-path $(WORKSPACE_MANIFEST) --workspace

generate-build: ## Build every release graph/metadata/EQ generator binary.
	$(CARGO) build --release --manifest-path $(WORKSPACE_MANIFEST) -p spinorama-generate --bins

renderer-build: ## Build the macOS release GPUI image renderer from GPUI_TOOLKIT_DIR.
	CARGO_TARGET_DIR="$(RENDER_TARGET_DIR)" $(CARGO) build --release --manifest-path "$(GPUI_TOOLKIT_DIR)/Cargo.toml" -p gpui-spinorama --bin spinorama-render-images --features capture-bench

graphs: generate-build ## Generate private graph payloads; optional SPEAKER and VERSION filters.
	"$(GENERATE_GRAPH_BIN)" --metadata-dir "$(METADATA_DIR)" --measurements-root "$(MEASUREMENTS_ROOT)" $(GRAPH_SELECTION) --jobs "$(JOBS)"

images: generate-build renderer-build ## Rebuild both binaries and regenerate PNG/WebP; optional SPEAKER and VERSION.
	"$(GENERATE_GRAPH_BIN)" --metadata-dir "$(METADATA_DIR)" --measurements-root "$(MEASUREMENTS_ROOT)" $(GRAPH_SELECTION) --jobs "$(JOBS)" --render-images-bin "$(RENDER_IMAGES_BIN)" --render-png --render-webp --force

check-meta: generate-build ## Validate speaker metadata with the Rust generator.
	"$(GENERATE_GRAPH_BIN)" --metadata-dir "$(METADATA_DIR)" --check-meta

check-meta-files: generate-build ## Validate metadata and load measurement files; optional SPEAKER and VERSION filters.
	"$(GENERATE_GRAPH_BIN)" --metadata-dir "$(METADATA_DIR)" --check-meta --check-files $(GRAPH_SELECTION)

check-public-dist: ## Reject private graph data in the public website tree.
	$(PYTHON) scripts/check_public_dist.py dist

api-check: ## Type-check the Rust API server crate.
	$(CARGO) check --manifest-path crates/spinorama-api/Cargo.toml

api-fmt: ## Check Rust API server formatting without modifying files.
	$(CARGO) fmt --manifest-path crates/spinorama-api/Cargo.toml -- --check

api-clippy: ## Lint the Rust API server and deny warnings.
	$(CARGO) clippy --manifest-path crates/spinorama-api/Cargo.toml --all-targets -- -D warnings

api-test: ## Run Rust API server tests.
	$(CARGO) test --manifest-path crates/spinorama-api/Cargo.toml

api-build: ## Build the release spinorama-api binary.
	$(CARGO) build --release --manifest-path $(WORKSPACE_MANIFEST) -p spinorama-api

wasm: ## Build the optional legacy annotation WASM package.
	$(WASM_PACK) build $(ANNOTATIONS_CRATE) --target web --release --out-dir $(WASM_BUILD_DIR)
	mkdir -p $(WASM_DIST_DIR)
	cp $(WASM_BUILD_DIR)/annotations_rust.js $(WASM_DIST_DIR)/annotations_rust.js
	cp $(WASM_BUILD_DIR)/annotations_rust_bg.wasm $(WASM_DIST_DIR)/annotations_rust_bg.wasm

build: ## Build the complete optimized production website.
	$(PYTHON) scripts/generate_html.py --optim

build-dev: ## Build fast local assets and reuse existing speaker pages.
	$(PYTHON) scripts/generate_html.py --dev --sitedev=$(DEV_URL) --skip-speakers

build-dev-full: ## Build the complete local website, including speaker pages.
	$(PYTHON) scripts/generate_html.py --dev --sitedev=$(DEV_URL)

build-dev-site: ## Build optimized assets for dev.spinorama.org deployment.
	$(PYTHON) scripts/generate_html.py --dev --optim --sitedev=$(DEPLOY_DEV_URL)

rebuild: rust images ## Rebuild Rust binaries, renderer, graph images, and the complete local site.
	$(MAKE) build-dev-full

# update_dev.sh installs the dev nginx configuration before uploading assets.
deploy-dev: build-dev-site ## Build and deploy the dev website and its nginx policy (validates public files).
	sh scripts/update_dev.sh

deploy-nginx-dev: ## Install and validate only the dev nginx configuration.
	sh scripts/deploy_nginx_dev.sh

deploy-binaries-dev: ## Build remotely and deploy Rust binaries to dev (excludes the GPUI renderer).
	sh scripts/deploy_binaries.sh dev

deploy-binaries-prod: ## Build remotely and deploy Rust binaries to production (excludes the GPUI renderer).
	sh scripts/deploy_binaries.sh prod

deploy-binaries-all: ## Build remotely and deploy Rust binaries to dev and production.
	sh scripts/deploy_binaries.sh all

stage-api-prod: ## Stage private API data and nginx files on production; prints installation commands.
	bash scripts/update_api_ovh.sh

serve: ## Serve dist with CORS and disabled caching on DEV_HOST:DEV_PORT.
	@test -d dist || { echo "dist is missing; run 'make build-dev' first"; exit 1; }
	@echo "Serving $(CURDIR)/dist at $(DEV_URL)"
	@cd dist && $(PYTHON) $(CURDIR)/scripts/debug_server.py --ip=$(DEV_HOST) --port=$(DEV_PORT)

dev: build-dev serve ## Build local assets and start the development server.

run: dev ## Alias for the local build-and-run workflow.

test: test-rust test-python test-js ## Run Rust, Python, and frontend test suites.

test-rust: workspace-test test-annotations-rust ## Run workspace and annotation-solver Rust tests.

test-annotations-rust: ## Run Rust annotation solver unit tests.
	$(CARGO) test --manifest-path $(ANNOTATIONS_MANIFEST)

test-python: ## Run the complete Python test suite.
	PYTHONPATH=$(PYTHONPATH) $(PYTHON) -m pytest tests

test-js: ## Run the frontend Vitest suite once.
	$(NPM) test -- --run

test-dev: ## Run Vitest browser checks against dev; override SPINORAMA_TEST_URL for another site.
	$(NPM) run test:dev

test-annotations: test-annotations-rust ## Run focused native and Python annotation tests.
	PYTHONPATH=$(PYTHONPATH) $(PYTHON) -m unittest tests.test_spin_plot_annotations
	$(NPM) test -- --run src/website/annotation-layout.test.js

lint-python: ## Run CI-critical Ruff checks over Python source and tests.
	$(PYTHON) -m ruff check --select E9,F63,F7,F82 $(PYTHON_LINT_PATHS)

lint-python-strict: ## Run every Ruff rule configured in pyproject.toml.
	$(PYTHON) -m ruff check $(PYTHON_LINT_PATHS)

lint: workspace-fmt workspace-clippy rust-fmt rust-clippy lint-python ## Run Rust, Python, and JavaScript lint checks.
	$(NPM) run lint

check: lint test ## Run workspace, annotation, Python, and frontend checks.

annotations: rust-python ## Render one annotated CEA-2034 plot (requires SPEAKER=...).
	@test -n "$(SPEAKER)" || { echo 'Set SPEAKER="speaker name"'; exit 1; }
	$(PYTHON) scripts/plot_cea2034_annotations.py "$(SPEAKER)" --force

clean: ## Run the repository cleanup script.
	./scripts/cleanup.sh
