//! Mail-bearing UI events must not expose content through toolkit diagnostics.

use core::fmt;

#[derive(Clone)]
pub struct Private<T>(pub T);
impl<T> fmt::Debug for Private<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<private mail data>")
    }
}
