#[macro_export]
macro_rules! rustia_extension {
    ($namespace:tt { $($items:tt)* }) => {
        $crate::rustia_extension!(@accum $crate::RustiaExtension::new($namespace), $($items)*)
    };
    ({ $($items:tt)* }) => {
        $crate::rustia_extension!(@accum $crate::RustiaExtension::global(), $($items)*)
    };

    (@accum $extension:expr,) => {
        $extension
    };
    (@accum $extension:expr) => {
        $extension
    };

    (@accum $extension:expr, pure fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_pure(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, pure(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::Pure,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };

    (@accum $extension:expr, input fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_input(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, input(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::ReadsDeterministicInput,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };

    (@accum $extension:expr, slot_read fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_slot_read(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, slot_read(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::ReadsSlotState,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };

    (@accum $extension:expr, slot_write fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_slot_write(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, slot_write(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::WritesSlotEffect,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };

    (@accum $extension:expr, effect fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_effect(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, effect(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::EmitsDeterministicEffect,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };

    (@accum $extension:expr, nondeterministic fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_nondeterministic(
                stringify!($name),
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
    (@accum $extension:expr, nondeterministic(fuel = $fuel:expr) fn $name:ident ( $($arg:ident : $arg_ty:ty),* $(,)? ) -> $ret:ty $body:block $($rest:tt)*) => {
        $crate::rustia_extension!(
            @accum
            $extension.fn_with(
                stringify!($name),
                $fuel,
                $crate::OpCapability::NondeterministicEffect,
                |$($arg: $arg_ty),*| -> $ret { $body },
            ),
            $($rest)*
        )
    };
}
