namespace DotnetRs;

[Stub(InPlaceOf = "System.MulticastDelegate")]
public abstract class MulticastDelegate : Delegate
{
    private Delegate[] targets;

    public MulticastDelegate(object target, nint method) : base(target, method)
    {
        targets = [this];
    }

    public override bool HasSingleTarget => targets.Length == 1;

    // TODO: add stubs for multicast API
}