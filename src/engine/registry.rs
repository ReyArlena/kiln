//! The list of every test kiln knows about.

use std::sync::Arc;

use super::test::Test;

#[derive(Clone)]
pub struct Registry {
    tests: Vec<Arc<dyn Test>>,
}

impl Registry {
    /// All built-in tests, in display order.
    pub fn builtin() -> Self {
        Self {
            tests: crate::suite::all(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Test>> {
        self.tests.iter()
    }

    pub fn len(&self) -> usize {
        self.tests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Test>> {
        self.tests.iter().find(|t| t.info().id == id)
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.tests.iter().map(|t| t.info().id).collect()
    }
}
