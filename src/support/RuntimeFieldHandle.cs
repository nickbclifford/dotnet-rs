using System.Runtime.CompilerServices;
using System.Runtime.Serialization;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeFieldHandle")]
public struct RuntimeFieldHandle : IEquatable<RuntimeFieldHandle>, ISerializable
{
    [UsedImplicitly] private nint _value;
    public IntPtr Value => _value;
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern RuntimeFieldHandle(nint value);
    
    public override bool Equals(object? obj)
    {
        if (obj is RuntimeFieldHandle o)
        {
            return Equals(o);
        }
        return false;
    }
    public bool Equals(RuntimeFieldHandle other) => other._value == _value;
    public static bool operator ==(object left, RuntimeFieldHandle right) => right.Equals(left);
    public static bool operator !=(object left, RuntimeFieldHandle right) => !(left == right);
    public static bool operator ==(RuntimeFieldHandle left, object right) => left.Equals(right);
    public static bool operator !=(RuntimeFieldHandle left, object right) => !(left == right);
    
    public static RuntimeFieldHandle FromIntPtr(IntPtr value) => new (value);
    
    public override int GetHashCode() => _value.GetHashCode();
    
    // obsolete according to docs, so I won't bother
    public void GetObjectData(SerializationInfo info, StreamingContext context) { }

    public static IntPtr ToIntPtr(RuntimeFieldHandle rth) => rth._value;
}