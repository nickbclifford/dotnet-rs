using System.Runtime.Serialization;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Delegate")]
public abstract class Delegate : ICloneable, ISerializable
{
    [RuntimeSlot(RuntimeSlotId.DelegateTarget)]
    [UsedImplicitly] internal object? _target;
    [RuntimeSlot(RuntimeSlotId.DelegateMethodIndex)]
    [UsedImplicitly] internal nint _method;

    protected Delegate(object target, nint method)
    {
        _target = target;
        _method = method;
    }

    public extern object? Target { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public virtual bool HasSingleTarget => true;

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern override bool Equals(object? obj);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern override int GetHashCode();

    // Routes through the runtime's reflection invoke path, so argument conversion, by-ref write-back
    // and TargetInvocationException wrapping match MethodInfo.Invoke. A multicast delegate invokes
    // every entry and returns the last result, as in the CLR.
    public object? DynamicInvoke(params object?[]? args)
    {
        object? result = null;
        foreach (Delegate entry in GetInvocationList())
        {
            result = entry.DynamicInvokeSingle(args);
        }
        return result;
    }

    public virtual Delegate[] GetInvocationList() => new Delegate[] { this };

    private object? DynamicInvokeSingle(object?[]? args)
    {
        System.Reflection.MethodInfo method = Method;
        object? target = Target;
        if (!method.IsStatic)
        {
            return method.Invoke(target, args);
        }
        if (target == null)
        {
            return method.Invoke(null, args);
        }
        // A static method closed over its first argument receives the target as that argument.
        int count = args == null ? 0 : args.Length;
        object?[] closed = new object?[count + 1];
        closed[0] = target;
        if (count > 0)
        {
            System.Array.Copy(args!, 0, closed, 1, count);
        }
        return method.Invoke(null, closed);
    }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public static extern Delegate? Combine(Delegate? a, Delegate? b);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public static extern Delegate? Remove(Delegate? source, Delegate? value);

    protected virtual Delegate CombineImpl(Delegate? d) => throw new NotSupportedException();

    protected virtual Delegate? RemoveImpl(Delegate? d) => d == (object)this ? null : this;
    
    public extern System.Reflection.MethodInfo Method { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public object Clone() => MemberwiseClone();

    public void GetObjectData(SerializationInfo info, StreamingContext context) { }
}
