using System.Runtime.CompilerServices;
using System.Runtime.Serialization;

namespace DotnetRs;

[Stub(InPlaceOf = "System.RuntimeTypeHandle")]
public struct RuntimeTypeHandle : IEquatable<RuntimeTypeHandle>, ISerializable
{
    private nint _value;
    public IntPtr Value => _value;
    
    [MethodImpl(MethodImplOptions.InternalCall)]
    private extern RuntimeTypeHandle(nint value);
    
    public override bool Equals(object? obj)
    {
        if (obj is RuntimeTypeHandle o)
        {
            return Equals(o);
        }
        return false;
    }
    public bool Equals(RuntimeTypeHandle other) => other._value == _value;
    public static bool operator ==(object left, RuntimeTypeHandle right) => right.Equals(left);
    public static bool operator !=(object left, RuntimeTypeHandle right) => !(left == right);
    public static bool operator ==(RuntimeTypeHandle left, object right) => left.Equals(right);
    public static bool operator !=(RuntimeTypeHandle left, object right) => !(left == right);
    
    public static RuntimeTypeHandle FromIntPtr(IntPtr value) => new (value);
    
    public override int GetHashCode() => _value.GetHashCode();
    
    // obsolete according to docs, so I won't bother
    public void GetObjectData(SerializationInfo info, StreamingContext context) { }

    public static IntPtr ToIntPtr(RuntimeTypeHandle rth) => rth._value;
}