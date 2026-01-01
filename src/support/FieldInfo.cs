using System.Reflection;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

public class FieldInfo : System.Reflection.FieldInfo
{
    [UsedImplicitly] private nint index;

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object[] GetCustomAttributes(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern bool IsDefined(Type attributeType, bool inherit);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type GetDeclaringType();
    public override Type DeclaringType => GetDeclaringType();
    public override Type ReflectedType => GetDeclaringType();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern string GetName();
    public override string Name => GetName();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern object? GetValue(object? obj);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public override extern void SetValue(object? obj, object? value, BindingFlags invokeAttr, Binder? binder, System.Globalization.CultureInfo? culture);

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern FieldAttributes GetFieldAttributes();
    public override FieldAttributes Attributes => GetFieldAttributes();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern System.RuntimeFieldHandle GetFieldHandle();
    public override System.RuntimeFieldHandle FieldHandle => GetFieldHandle();

    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern Type GetFieldType();
    public override Type FieldType => GetFieldType();
}
