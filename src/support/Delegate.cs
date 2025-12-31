using System.Runtime.Serialization;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Delegate")]
public abstract class Delegate(object target, nint method) : ICloneable, ISerializable
{
    public object? Target { get; } = target;
    private RuntimeMethodHandle _method = new(method);

    public virtual bool HasSingleTarget => true;
    
    public System.Reflection.MethodInfo Method => throw new NotImplementedException();

    // TODO: add stubs for rest of public Delegate API
    
    public object Clone() => MemberwiseClone();

    public void GetObjectData(SerializationInfo info, StreamingContext context) { }
}