use pyo3::inspect::PyStaticExpr;
use pyo3::prelude::*;
use pyo3::type_hint_identifier;

macro_rules! string_enum_output {
    ($($name:ident),+ $(,)?) => {
        $(
            pub(crate) struct $name(pub &'static str);

            impl<'py> IntoPyObject<'py> for $name {
                type Target = PyAny;
                type Output = Bound<'py, PyAny>;
                type Error = PyErr;

                const OUTPUT_TYPE: PyStaticExpr =
                    type_hint_identifier!("rusty_desktop_icons._enums", stringify!($name));

                fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
                    py.import("rusty_desktop_icons._enums")?
                        .getattr(stringify!($name))?
                        .call1((self.0,))
                }
            }
        )+
    };
}

string_enum_output!(CurveKind, DurationKind, FinishReasonKind, FolderFlagOpKind, ShaderPipeline);