
namespace DotnetRs;

/// <summary>
/// Object-array delegate thunks for System.Linq.Expressions' interpreter. NativeAOT supplies this
/// through <c>DynamicDelegateAugments.CreateObjectArrayDelegate</c>; CoreCLR's CoreLib has no such
/// type and emits a <c>DynamicMethod</c> for delegates with more than two parameters. These generic
/// thunks pack the arguments into an <c>object[]</c>, call the handler and cast the result, the
/// same shape the Emit thunk has, for delegates of up to 16 parameters without byref, pointer or
/// byref-like types. The VM routes <c>DelegateHelpers.CreateObjectArrayDelegate</c> here.
/// </summary>
public static class ObjectArrayDelegates
{
    private const int MaxParameters = 16;

    private static readonly object s_lock = new object();
    private static readonly Dictionary<Type, System.Reflection.MethodInfo> s_thunks = new Dictionary<Type, System.Reflection.MethodInfo>();

    public static System.Delegate Create(Type delegateType, Func<object[], object> handler)
    {
        System.Reflection.MethodInfo? thunk;
        lock (s_lock)
        {
            s_thunks.TryGetValue(delegateType, out thunk);
        }

        if (thunk == null)
        {
            thunk = FindThunk(delegateType);
            lock (s_lock)
            {
                s_thunks[delegateType] = thunk;
            }
        }

        return thunk.CreateDelegate(delegateType, handler);
    }

    private static System.Reflection.MethodInfo FindThunk(Type delegateType)
    {
        System.Reflection.MethodInfo invoke = delegateType.GetMethod("Invoke")!;
        Type returnType = invoke.ReturnType;
        bool hasReturn = returnType != typeof(void);
        System.Reflection.ParameterInfo[] parameters = invoke.GetParameters();
        if (parameters.Length > MaxParameters || IsUnsupported(returnType))
        {
            throw new PlatformNotSupportedException();
        }

        Type[] typeArguments = new Type[parameters.Length + (hasReturn ? 1 : 0)];
        for (int i = 0; i < parameters.Length; i++)
        {
            Type parameterType = parameters[i].ParameterType;
            if (IsUnsupported(parameterType))
            {
                throw new PlatformNotSupportedException();
            }

            typeArguments[i] = parameterType;
        }

        if (hasReturn)
        {
            typeArguments[parameters.Length] = returnType;
        }

        string name = (hasReturn ? "FuncThunk" : "ActionThunk") + parameters.Length;
        System.Reflection.MethodInfo thunk = typeof(ObjectArrayDelegates).GetMethod(name)!;
        return typeArguments.Length == 0 ? thunk : thunk.MakeGenericMethod(typeArguments);
    }

    private static bool IsUnsupported(Type type) => type.IsByRef || type.IsPointer || type.IsByRefLike;
    public static void ActionThunk0(Func<object[], object> handler) => handler(Array.Empty<object>());
    public static void ActionThunk1<T1>(Func<object[], object> handler, T1 t1) => handler(new object[1] { t1 });
    public static void ActionThunk2<T1, T2>(Func<object[], object> handler, T1 t1, T2 t2) => handler(new object[2] { t1, t2 });
    public static void ActionThunk3<T1, T2, T3>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3) => handler(new object[3] { t1, t2, t3 });
    public static void ActionThunk4<T1, T2, T3, T4>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4) => handler(new object[4] { t1, t2, t3, t4 });
    public static void ActionThunk5<T1, T2, T3, T4, T5>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5) => handler(new object[5] { t1, t2, t3, t4, t5 });
    public static void ActionThunk6<T1, T2, T3, T4, T5, T6>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6) => handler(new object[6] { t1, t2, t3, t4, t5, t6 });
    public static void ActionThunk7<T1, T2, T3, T4, T5, T6, T7>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7) => handler(new object[7] { t1, t2, t3, t4, t5, t6, t7 });
    public static void ActionThunk8<T1, T2, T3, T4, T5, T6, T7, T8>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8) => handler(new object[8] { t1, t2, t3, t4, t5, t6, t7, t8 });
    public static void ActionThunk9<T1, T2, T3, T4, T5, T6, T7, T8, T9>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9) => handler(new object[9] { t1, t2, t3, t4, t5, t6, t7, t8, t9 });
    public static void ActionThunk10<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10) => handler(new object[10] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10 });
    public static void ActionThunk11<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11) => handler(new object[11] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11 });
    public static void ActionThunk12<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12) => handler(new object[12] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12 });
    public static void ActionThunk13<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13) => handler(new object[13] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13 });
    public static void ActionThunk14<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14) => handler(new object[14] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14 });
    public static void ActionThunk15<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14, T15 t15) => handler(new object[15] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15 });
    public static void ActionThunk16<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15, T16>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14, T15 t15, T16 t16) => handler(new object[16] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15, t16 });
    public static TReturn FuncThunk0<TReturn>(Func<object[], object> handler) => (TReturn)handler(Array.Empty<object>());
    public static TReturn FuncThunk1<T1, TReturn>(Func<object[], object> handler, T1 t1) => (TReturn)handler(new object[1] { t1 });
    public static TReturn FuncThunk2<T1, T2, TReturn>(Func<object[], object> handler, T1 t1, T2 t2) => (TReturn)handler(new object[2] { t1, t2 });
    public static TReturn FuncThunk3<T1, T2, T3, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3) => (TReturn)handler(new object[3] { t1, t2, t3 });
    public static TReturn FuncThunk4<T1, T2, T3, T4, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4) => (TReturn)handler(new object[4] { t1, t2, t3, t4 });
    public static TReturn FuncThunk5<T1, T2, T3, T4, T5, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5) => (TReturn)handler(new object[5] { t1, t2, t3, t4, t5 });
    public static TReturn FuncThunk6<T1, T2, T3, T4, T5, T6, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6) => (TReturn)handler(new object[6] { t1, t2, t3, t4, t5, t6 });
    public static TReturn FuncThunk7<T1, T2, T3, T4, T5, T6, T7, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7) => (TReturn)handler(new object[7] { t1, t2, t3, t4, t5, t6, t7 });
    public static TReturn FuncThunk8<T1, T2, T3, T4, T5, T6, T7, T8, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8) => (TReturn)handler(new object[8] { t1, t2, t3, t4, t5, t6, t7, t8 });
    public static TReturn FuncThunk9<T1, T2, T3, T4, T5, T6, T7, T8, T9, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9) => (TReturn)handler(new object[9] { t1, t2, t3, t4, t5, t6, t7, t8, t9 });
    public static TReturn FuncThunk10<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10) => (TReturn)handler(new object[10] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10 });
    public static TReturn FuncThunk11<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11) => (TReturn)handler(new object[11] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11 });
    public static TReturn FuncThunk12<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12) => (TReturn)handler(new object[12] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12 });
    public static TReturn FuncThunk13<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13) => (TReturn)handler(new object[13] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13 });
    public static TReturn FuncThunk14<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14) => (TReturn)handler(new object[14] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14 });
    public static TReturn FuncThunk15<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14, T15 t15) => (TReturn)handler(new object[15] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15 });
    public static TReturn FuncThunk16<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15, T16, TReturn>(Func<object[], object> handler, T1 t1, T2 t2, T3 t3, T4 t4, T5 t5, T6 t6, T7 t7, T8 t8, T9 t9, T10 t10, T11 t11, T12 t12, T13 t13, T14 t14, T15 t15, T16 t16) => (TReturn)handler(new object[16] { t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15, t16 });
}
