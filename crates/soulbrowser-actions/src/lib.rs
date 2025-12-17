//! SoulBrowser actions prelude.

pub mod primitives {
    pub mod errors {
        pub use action_primitives::errors::*;
    }
    pub mod types {
        pub use action_primitives::types::*;
    }

    pub use action_primitives::{ActionPrimitives, DefaultActionPrimitives};
    pub use action_primitives::{
        AnchorDescriptor, ExecCtx, ScrollBehavior, ScrollTarget, SelectMethod,
    };
    pub use action_primitives::{WaitCondition, WaitTier};
}

pub mod locator {
    pub mod errors {
        pub use action_locator::errors::*;
    }
    pub mod healer {
        pub use action_locator::healer::*;
    }
    pub mod resolver {
        pub use action_locator::resolver::*;
    }
    pub mod strategies {
        pub use action_locator::strategies::*;
    }
    pub mod types {
        pub use action_locator::types::*;
    }
}

pub mod gate {
    pub mod errors {
        pub use action_gate::errors::*;
    }
    pub mod evidence {
        pub use action_gate::evidence::*;
    }
    pub mod types {
        pub use action_gate::conditions::*;
        pub use action_gate::types::*;
    }
    pub mod validator {
        pub use action_gate::validator::*;
    }
}

pub mod flow {
    pub mod errors {
        pub use action_flow::errors::*;
    }
    pub mod executor {
        pub use action_flow::executor::*;
    }
    pub mod strategies {
        pub use action_flow::strategies::*;
    }
    pub mod types {
        pub use action_flow::types::*;
    }
}
