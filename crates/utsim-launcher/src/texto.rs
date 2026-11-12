//! The launcher's texts in the five languages of the family: the keys of
//! `idiomas/<código>/lancador.ftl` (`docs/traducao.md`). The window writes no literal text:
//! it reads `tx("chave")` or `fx("chave", &[("nome", vx(&valor))])`.

use std::fmt::Display;

use utsim_idiomas::{Catalogo, Valor};

/// The launcher's texts in the five languages, embedded from `idiomas/`.
pub static TEXTOS: Catalogo = utsim_idiomas::catalogo!("lancador");

/// The text of a key in the language of the moment.
pub fn tx(chave: &'static str) -> &'static str {
    TEXTOS.t(chave)
}

/// The text of a key with named values.
pub fn fx(chave: &str, args: &[(&str, Valor)]) -> String {
    TEXTOS.f(chave, args)
}

/// A value: a number when it reads back the same (plurals pick by it), else text.
pub fn vx(x: &impl Display) -> Valor<'static> {
    let s = x.to_string();
    if let Ok(n) = s.parse::<i64>()
        && n.to_string() == s
    {
        return n.into();
    }
    if let Ok(f) = s.parse::<f64>()
        && f.is_finite()
        && f.to_string() == s
    {
        return f.into();
    }
    s.into()
}

/// A number already with fixed places (`{:.0}`), with the language's decimal mark.
pub fn dx(s: String) -> Valor<'static> {
    utsim_idiomas::marca_decimal(&s).into()
}

/// A text as it is.
pub fn sx(s: String) -> Valor<'static> {
    s.into()
}
