//! Compile-only regressions for exact payload/schema brands after type erasure.
//! A function accepting any string lifetime is a Rust subtype of one accepting
//! only static strings, but these VM registrations must never change brands.

/// ```compile_fail
/// fn rebrand(value: rustia::Object<fn(&str)>) -> rustia::Object<fn(&'static str)> {
///     value
/// }
/// ```
/// ```compile_fail
/// fn rebrand(value: rustia::ObjectType<fn(&str)>) -> rustia::ObjectType<fn(&'static str)> {
///     value
/// }
/// ```
/// ```compile_fail
/// use rustia::HostSchema;
/// struct Schema<T>(std::marker::PhantomData<T>);
/// impl<T: 'static> HostSchema for Schema<T> {
///     type Input = (); type Slots = (); type External = ();
///     type Effect = (); type SlotWrite = (); type Record = ();
/// }
/// fn rebrand(value: rustia::HostFunction<Schema<fn(&str)>>) -> rustia::HostFunction<Schema<fn(&'static str)>> {
///     value
/// }
/// ```
/// ```compile_fail
/// use rustia::HostSchema;
/// struct Schema<T>(std::marker::PhantomData<T>);
/// impl<T: 'static> HostSchema for Schema<T> {
///     type Input = (); type Slots = (); type External = ();
///     type Effect = (); type SlotWrite = (); type Record = ();
/// }
/// fn rebrand(value: rustia::HostModule<Schema<fn(&str)>>) -> rustia::HostModule<Schema<fn(&'static str)>> {
///     value
/// }
/// ```
/// ```compile_fail
/// use rustia::HostSchema;
/// struct Schema<T>(std::marker::PhantomData<T>);
/// impl<T: 'static> HostSchema for Schema<T> {
///     type Input = (); type Slots = (); type External = ();
///     type Effect = (); type SlotWrite = (); type Record = ();
/// }
/// fn rebrand(value: rustia::ObjectType<(), Schema<fn(&str)>>) -> rustia::ObjectType<(), Schema<fn(&'static str)>> {
///     value
/// }
/// ```
#[allow(dead_code)]
pub(crate) const EXACT_ERASED_BRANDS: () = ();
