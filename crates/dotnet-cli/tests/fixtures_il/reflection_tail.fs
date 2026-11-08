open System
open System.Reflection
open System.Runtime.CompilerServices
open System.Runtime.InteropServices

type NativeMethods =
    [<DllImport("libc", EntryPoint = "abs")>]
    static extern int NativeAbs(int value)

[<MethodImpl(MethodImplOptions.NoInlining)>]
let invoke (methodInfo: MethodInfo) (target: obj) (args: obj array) : obj =
    methodInfo.Invoke(target, BindingFlags.Default, null, args, null)

[<MethodImpl(MethodImplOptions.NoInlining)>]
let invokeCtor (constructor: ConstructorInfo) (args: obj array) : obj =
    constructor.Invoke(BindingFlags.Default, null, args, null)

[<MethodImpl(MethodImplOptions.NoInlining)>]
let create (target: Type) (args: obj array) : obj =
    Activator.CreateInstance(target, args)

type Target =
    static let mutable count = 0
    static do count <- 40
    static member Next() = count <- count + 1; count
    static member Text() = "value"
    static member Nothing() = ()
    static member Throw() : int = raise (InvalidOperationException())

type NestedTarget =
    static let mutable value = 0
    static do value <- 42
    static member Get() = value

type RefThing(value: int) = member _.Value = value

type ThrowingThing(value: int) =
    do if value = 42 then raise (InvalidOperationException())

[<Struct>]
type ValueThing =
    val Value: int
    new (value: int) = { Value = value }

[<EntryPoint>]
let main _ =
    let mi = typeof<Math>.GetMethod("Abs", [| typeof<int> |])
    let result = invoke mi null [| box -42 |]
    if unbox<int> result <> 42 then 1 else
    let ty = typeof<Target>
    let next = ty.GetMethod("Next")
    if unbox<int> (invoke next null null) <> 41 then 2 else
    if unbox<string> (invoke (ty.GetMethod("Text")) null null) <> "value" then 3 else
    if invoke (ty.GetMethod("Nothing")) null null <> null then 4 else
    let caught = try invoke (ty.GetMethod("Throw")) null null |> ignore; false with _ -> true
    if not caught then 5 else
    let substring = typeof<string>.GetMethod("Substring", [| typeof<int> |])
    let caughtIntrinsic = try invoke substring (box "hello") [| box 20 |] |> ignore; false with _ -> true
    if not caughtIntrinsic then 9 else
    if unbox<int> (invoke mi null [| box -42 |]) <> 42 then 10 else
    let native = typeof<NativeMethods>.GetMethod("NativeAbs", BindingFlags.Static ||| BindingFlags.Public ||| BindingFlags.NonPublic)
    if unbox<int> (invoke native null [| box -42 |]) <> 42 then 11 else
    if unbox<int> (invoke next null null) <> 42 then 6 else
    let inner = typeof<NestedTarget>.GetMethod("Get")
    let invokeTypes = [| typeof<obj>; typeof<BindingFlags>; typeof<Binder>; typeof<obj array>; typeof<System.Globalization.CultureInfo> |]
    let outer = typeof<MethodBase>.GetMethod("Invoke", invokeTypes)
    let nested = invoke outer (box inner) [| null; box BindingFlags.Default; null; null; null |]
    if unbox<int> nested <> 42 then 8 else
    let types = [| typeof<int> |]
    let values = [| box 42 |]
    let c1 = invokeCtor (typeof<RefThing>.GetConstructor(types)) values :?> RefThing
    let existing = RefThing(1)
    (typeof<RefThing>.GetConstructor(types) :> MethodBase).Invoke(existing, BindingFlags.Default, null, values, null) |> ignore
    if existing.Value <> 42 then failwith "constructor did not update its receiver"
    let c2 = invokeCtor (typeof<ValueThing>.GetConstructor(types)) values |> unbox<ValueThing>
    let c3 = create typeof<RefThing> values :?> RefThing
    let c4 = create typeof<ValueThing> values |> unbox<ValueThing>
    let badCtor = typeof<ThrowingThing>.GetConstructor(types)
    let caughtCtor = try invokeCtor badCtor values |> ignore; false with _ -> true
    let caughtActivator = try create typeof<ThrowingThing> values |> ignore; false with _ -> true
    if c1.Value = 42 && c2.Value = 42 && c3.Value = 42 && c4.Value = 42 && caughtCtor && caughtActivator
    then 42 else 7
