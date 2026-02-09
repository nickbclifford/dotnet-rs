using System.Runtime.Serialization;
using System.Runtime.CompilerServices;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Delegate")]
public abstract class Delegate : ICloneable, ISerializable
{
    internal object? _target;
    internal RuntimeMethodHandle _method;

    protected Delegate(object target, nint method)
    {
        _target = target;
        _method = new RuntimeMethodHandle(method);
    }

    public extern object? Target { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public virtual bool HasSingleTarget => true;

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern override bool Equals(object? obj);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern override int GetHashCode();

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern object? DynamicInvoke(params object?[]? args);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public static extern Delegate? Combine(Delegate? a, Delegate? b);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public static extern Delegate? Remove(Delegate? source, Delegate? value);

    protected virtual Delegate CombineImpl(Delegate? d) => throw new System.NotSupportedException();

    protected virtual Delegate? RemoveImpl(Delegate? d) => d == (object)this ? null : this;
    
    public extern System.Reflection.MethodInfo Method { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public object Clone() => MemberwiseClone();

    public void GetObjectData(SerializationInfo info, StreamingContext context) { }
}