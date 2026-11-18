using System;
using System.Reflection;
using System.Reflection.Emit;
using System.Runtime.CompilerServices;
using System.Threading;

interface ICanonicalShape { int Code(); }
class CanonicalBase<T>
{
    public virtual T Echo(T value) => value;
}
class CanonicalTarget<T> : CanonicalBase<T>, ICanonicalShape
{
    public T Field = default!;
    public T Property { get; set; } = default!;
    public override T Echo(T value) => value;
    public U Convert<U>(U value) => value;
    public int Code() => 42;
}
sealed class Probe { public readonly object Value; public Probe(object value) { Value = value; } }

static class Program
{
    static Type? workerType;
    static MethodInfo? workerEcho, workerBaseEcho, workerGeneric, workerDelegateMethod;
    static FieldInfo? workerField;
    static PropertyInfo? workerProperty, workerEnumeratedProperty;
    static Assembly? workerAssembly;
    static WeakReference? workerMethodWeak;
    static readonly ConditionalWeakTable<Type, Probe> roots = new ConditionalWeakTable<Type, Probe>();
    static bool workerOk;

    static PropertyInfo? FindProperty(Type type)
    {
        foreach (PropertyInfo property in type.GetProperties())
            if (property.Name == "Property") return property;
        return null;
    }

    static void Work()
    {
        Type type = typeof(CanonicalTarget<int>);
        MethodInfo echo = type.GetMethod("Echo")!;
        MethodInfo baseEcho = echo.GetBaseDefinition();
        MethodInfo generic = type.GetMethod("Convert")!.MakeGenericMethod(typeof(string));
        FieldInfo field = type.GetField("Field")!;
        PropertyInfo property = type.GetProperty("Property")!;
        PropertyInfo enumerated = FindProperty(type)!;
        bool shape = type.GetInterfaces().Length == 1
            && type.GetInterface(typeof(ICanonicalShape).Name) == typeof(ICanonicalShape)
            && typeof(CanonicalTarget<>).GetGenericTypeDefinition() == typeof(CanonicalTarget<>)
            && typeof(CanonicalTarget<>).GetGenericArguments().Length == 1
            && typeof(int[]).GetElementType() == typeof(int)
            && generic.GetParameters()[0].ParameterType == typeof(string)
            && generic.ReturnType == typeof(string)
            && baseEcho.DeclaringType == typeof(CanonicalBase<int>)
            && property.PropertyType == typeof(int)
            && property.DeclaringType == type
            && property.GetGetMethod() != null
            && type.GetMember("Echo").Length != 0;

        var target = new CanonicalTarget<int>();
        var callback = (Func<int, int>)echo.CreateDelegate(typeof(Func<int, int>), target);
        MethodInfo delegateMethod = callback.Method;
        var dynamicMethod = new DynamicMethod("CanonicalRecorded", typeof(int), Type.EmptyTypes);
        ILGenerator il = dynamicMethod.GetILGenerator();
        il.Emit(OpCodes.Ldc_I4, 42);
        il.Emit(OpCodes.Ret);
        var dynamicCall = (Func<int>)dynamicMethod.CreateDelegate(typeof(Func<int>));

        workerType = type;
        workerEcho = echo;
        workerBaseEcho = baseEcho;
        workerGeneric = generic;
        workerField = field;
        workerProperty = property;
        workerEnumeratedProperty = enumerated;
        workerAssembly = type.Assembly;
        workerDelegateMethod = delegateMethod;
        workerMethodWeak = new WeakReference(echo);
        roots.Add(type, new Probe(property));
        workerOk = shape && callback(42) == 42 && dynamicCall() == 42
            && ReferenceEquals(delegateMethod, echo);
    }

    static int Main()
    {
        var worker = new Thread(Work);
        worker.Start();
        worker.Join();
        GC.Collect();
        GC.WaitForPendingFinalizers();
        GC.Collect();

        Type type = typeof(CanonicalTarget<int>);
        MethodInfo echo = type.GetMethod("Echo")!;
        PropertyInfo property = type.GetProperty("Property")!;
        MethodInfo generic = type.GetMethod("Convert")!.MakeGenericMethod(typeof(string));
        bool ok = workerOk
            && ReferenceEquals(type, workerType)
            && ReferenceEquals(echo, workerEcho)
            && ReferenceEquals(echo.GetBaseDefinition(), workerBaseEcho)
            && ReferenceEquals(generic, workerGeneric)
            && ReferenceEquals(type.GetField("Field"), workerField)
            && ReferenceEquals(property, workerProperty)
            && ReferenceEquals(FindProperty(type), workerEnumeratedProperty)
            && ReferenceEquals(type.Assembly, workerAssembly)
            && ReferenceEquals(echo, workerDelegateMethod)
            && workerMethodWeak!.IsAlive
            && roots.TryGetValue(type, out Probe? probe)
            && ReferenceEquals(probe!.Value, property);
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
