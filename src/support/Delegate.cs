using System.Runtime.CompilerServices;
using System.Runtime.Serialization;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Delegate")]
public abstract class Delegate : ICloneable, ISerializable
{
    private object? target;
    private RuntimeMethodHandle method;

    public Delegate(object target, nint method)
    {
        this.target = target;
        this.method = new RuntimeMethodHandle(method);
    }

    public virtual bool HasSingleTarget => true;
    
    public System.Reflection.MethodInfo Method => throw new NotImplementedException();
    
    public object? Target => target;
    
    // TODO: add stubs for rest of public Delegate API
    
    public object Clone() => MemberwiseClone();

    public void GetObjectData(SerializationInfo info, StreamingContext context) { }
}