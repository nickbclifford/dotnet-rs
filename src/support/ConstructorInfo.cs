using System.Globalization;
using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

public class ConstructorInfo : System.Reflection.ConstructorInfo
{
    [UsedImplicitly] private nint index;

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern bool IsDefined(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type? GetDeclaringType();
    public override Type? DeclaringType => GetDeclaringType();
    public override Type? ReflectedType => GetDeclaringType();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern MethodImplAttributes GetMethodImplementationFlags();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern ParameterInfo[] GetParameters();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object? Invoke(object? obj, BindingFlags invokeAttr, Binder? binder, object?[]? parameters, CultureInfo? culture);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object Invoke(BindingFlags invokeAttr, Binder? binder, object?[]? parameters, CultureInfo? culture);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern MethodAttributes GetMethodFlags();
    public override MethodAttributes Attributes => GetMethodFlags();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern System.RuntimeMethodHandle GetMethodHandle();
    public override System.RuntimeMethodHandle MethodHandle => GetMethodHandle();
}
