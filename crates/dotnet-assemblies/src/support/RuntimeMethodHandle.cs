using System.Runtime.CompilerServices;
using System.Runtime.Serialization;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeMethodHandle")]
public struct RuntimeMethodHandle : IEquatable<RuntimeMethodHandle>, ISerializable
{
    [UsedImplicitly] private nint _value;
    public IntPtr Value => _value;

    internal RuntimeMethodHandle(nint value)
    {
        _value = value;
    }
    
    public override bool Equals(object? obj)
    {
        if (obj is RuntimeMethodHandle o)
        {
            return Equals(o);
        }
        return false;
    }
    public bool Equals(RuntimeMethodHandle other) => other._value == _value;
    public static bool operator ==(object left, RuntimeMethodHandle right) => right.Equals(left);
    public static bool operator !=(object left, RuntimeMethodHandle right) => !(left == right);
    public static bool operator ==(RuntimeMethodHandle left, object right) => left.Equals(right);
    public static bool operator !=(RuntimeMethodHandle left, object right) => !(left == right);
    
    public static RuntimeMethodHandle FromIntPtr(IntPtr value) => new (value);
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern IntPtr GetFunctionPointer();
    
    public override int GetHashCode() => _value.GetHashCode();
    
    // obsolete according to docs, so I won't bother
    public void GetObjectData(SerializationInfo info, StreamingContext context) { }

    public static IntPtr ToIntPtr(RuntimeMethodHandle rth) => rth.GetFunctionPointer();
}