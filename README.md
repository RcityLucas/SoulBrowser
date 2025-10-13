# SoulBrowser 1.0 - Intelligent Browser Automation Framework

> A living, breathing browser automation system that perceives, understands, and acts with intelligence.

## 🌟 Philosophy

SoulBrowser isn't just another automation tool - it's a digital consciousness that navigates the web with awareness and purpose. Built with Rust for performance and safety, it treats web interactions as a journey through digital realms, where every element has multiple identities and every action creates ripples of change.

## 🏗️ Architecture

```
L7: External Interfaces [Later]
    └── HTTP/gRPC/MCP Adapters
L6: Governance & Observability [MVP]
    └── Metrics, Timeline, Privacy
L5: Tools Layer [MVP]
    └── 12 High-level Tools
L4: Elastic Persistence [MVP/Scaffold]
    └── Event Store, Snapshots
L3: Intelligent Action [MVP]
    └── Primitives, Locator, Validation
L2: Layered Perception [MVP/Scaffold]
    └── Structural, Visual, Semantic
L1: Unified Kernel [Next]
    └── Dispatcher, Scheduler, State (development kicking off)
L0: Runtime & Adapters [In-flight]
    └── CDP scaffolding, Permissions, Network, Stealth, Extensions

## ✅ Current Status

- **L0 Runtime & Adapters**: CDP adapter now exposes a pluggable transport/event loop; permissions broker, network tap (light), stealth runtime, and extensions bridge each ship with in-memory runtimes and crate-level smoke tests.
- **Legacy layers (L1+)**: existing soul-base wiring remains in place for the CLI; fresh L1 work (dispatcher/scheduler/state) begins next.
- **Scheduler telemetry**: the CLI embeds the unified kernel scheduler; `soulbrowser info` and `soulbrowser scheduler` surface recent dispatch attempts, timestamps, basic Registry lifecycle events, and support cancelling pending actions.
- **Policy center**: `soulbrowser policy show`/`override` exposes current limits and allows runtime overrides (with TTL) that feed into the scheduler/registry.
- **Feature flags**: legacy examples/tests are gated behind `legacy-examples` / `legacy-tests` to keep the default build green.

See `docs/l0_development_plan.md` for detailed progress and `docs/l1_development_plan.md` for the upcoming kernel roadmap.
```

## 🚀 Quick Start

```bash
# Clone the repository
git clone https://github.com/yourusername/soulbrowser
cd soulbrowser

# Build the project
cargo build

# Run tests
cargo test

# Run legacy examples (feature-gated)
cargo run --features legacy-examples --example basic_demo
```

## 📦 Project Structure

```
SoulBrowser/
├── crates/
│   ├── cdp-adapter/         # L0 transport & event wiring scaffold
│   ├── permissions-broker/  # L0 policy runtime with TTL-aware decisions
│   ├── network-tap-light/   # L0 network summary & snapshot helper
│   ├── stealth/             # L0 fingerprint & captcha runtime scaffold
│   └── extensions-bridge/   # L0 MV3 bridge scaffold
├── docs/                    # Architecture plans & progress notes
├── examples/                # Legacy demos (enable with `legacy-examples` feature)
├── src/                     # CLI entrypoint and soul-base orchestration
├── tests/                   # Integration harness (legacy bits gated)
└── target/                  # Cargo build artifacts
```

## 🔑 Key Features

### Multi-Modal Element Targeting
- **CSS Selectors** - Traditional web selectors
- **ARIA Attributes** - Accessibility-based targeting
- **Text Content** - Find by visible text
- **Geometric Position** - Coordinate-based location
- **Intelligent Fallback** - Automatic strategy switching

### Unified Observation System
Every action produces a rich observation containing:
- Multi-dimensional signals (DOM, Network, Console)
- Success/failure status with detailed context
- Performance metrics and timing
- Optional artifacts (screenshots, snapshots)

### Intelligent Recovery
- Automatic retry with exponential backoff
- Context-aware recovery strategies
- Graceful degradation under pressure
- Self-healing element locators

## 🛠️ Core Components

### L0: Runtime & Adapters
- **CDP Adapter**: Pluggable transport + cancellable event loop (real CDP wiring in progress)
- **Permissions Broker**: In-memory policy store with TTL-aware decisions
- **Network Tap (Light)**: Per-page snapshot & summary registry, awaiting CDP feed
- **Stealth Runtime**: Profile catalog, applied-profile cache, captcha hooks
- **Extensions Bridge**: MV3 channel runtime (enable/disable, open/invoke stubs)

### L1: Unified Kernel
- **Session Registry**: Multi-session lifecycle management
- **Task Dispatcher**: Intelligent task routing
- **Execution Scheduler**: Concurrency and priority control
- **State Center**: In-memory state with event logging

### L2: Layered Perception
- **Structural Perceiver**: DOM and Accessibility tree analysis
- **Visual Perceiver**: Screenshot and OCR capabilities [Scaffold]
- **Semantic Perceiver**: Content understanding [Later]
- **Runtime Perceiver**: Console and performance monitoring

### L3: Intelligent Action
- **Action Primitives**: Low-level browser operations
- **Smart Locator**: Multi-strategy element finding with self-heal
- **Post-condition Gates**: Action validation and verification
- **Flow Orchestration**: Complex action sequences [Scaffold]

### L5: Tools Layer
High-level tools that combine perception, action, and validation:
- `navigate-to-url` - Smart navigation with verification
- `click` - Intelligent clicking with fallback
- `type-text` - Robust text input
- `wait-for-element` - Smart waiting strategies
- And 8 more specialized tools...

## 🔄 Data Flow

1. **Agent Request** → L5 Tool receives high-level command
2. **Perception** → L2 analyzes page and generates anchors
3. **Action** → L3 executes primitive with L0 capabilities
4. **Validation** → Post-condition gates verify success
5. **Observation** → Unified envelope returned to agent
6. **Persistence** → L4 stores events and snapshots
7. **Monitoring** → L6 tracks metrics and timeline

## 🧾 Automation Script DSL

The CLI `run` command understands a lightweight DSL for scripting browser flows. Core actions mirror CLI options (`navigate`, `click`, `type`, `wait`, `screenshot`), while control keywords unlock structured flows:

```
# substitute parameters defined via --param key=value or config
set greeting Hello World

loop 3
  navigate https://example.com
  type #search {{greeting}}
  click #submit
endloop

if environment == staging
  screenshot stage.png
else
  screenshot prod.png
endif

 parallel 2
   branch
     navigate https://example.com/profile
     wait 500
   endbranch
   branch
     navigate https://example.com/settings
     wait 500
   endbranch
 endparallel
```

 Loops accept numeric counts (or templated values), `if` supports `==` / `!=` comparisons against parameters or `set` locals, and `parallel` runs branches concurrently up to the configured `parallel_instances` limit (override per block with `parallel N`).

## 🧪 Development

```bash
# Run core test suites
cargo test
cargo test -p cdp-adapter
cargo test -p permissions-broker
cargo test -p network-tap-light
cargo test -p stealth
cargo test -p extensions-bridge

# Run with logging
RUST_LOG=debug cargo run

# Build documentation
cargo doc --open

# Format & lint
cargo fmt
cargo clippy
```

## 📊 Roadmap

### Phase 1: Foundation (Current)
- ✅ Core architecture setup
- ✅ Unified data contracts
- 🔄 L0-L1 implementation
- ⏳ L2-L3 core features

### Phase 2: Intelligence
- ⏳ Visual perception
- ⏳ Semantic understanding
- ⏳ Advanced recovery
- ⏳ Flow orchestration

### Phase 3: Scale
- ⏳ Distributed execution
- ⏳ Cloud deployment
- ⏳ External APIs
- ⏳ Plugin system

## ⚙️ Configuration

- **Policy files**: Edit `config/policies/browser_policy.json` or point `SOUL_POLICY_PATH` to a custom JSON file.
- **Strict authorization**: `SOUL_STRICT_AUTHZ=true` forces authorization decisions to respect the facade result without route-policy fallback.
- **Quota persistence**: Adjust `SOUL_QUOTA_PERSIST_MS` / `SOUL_QUOTA_REFRESH_MS` for disk sync and reload cadence.
- See `config/config.yaml.example` for a complete configuration template.
- For a deeper overview of the soul-base integration, see `docs/soul_base_components.md`.

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## 📄 License

MIT OR Apache-2.0

## 🌈 Philosophy Quote

> "Like a gardener tending to a digital garden, SoulBrowser nurtures each interaction, 
> understanding that every click creates ripples, every navigation opens new realms, 
> and every observation teaches us about the living web."

---

Built with ❤️ and Rust by the SoulBrowser Team
