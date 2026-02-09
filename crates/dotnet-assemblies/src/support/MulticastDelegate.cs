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

    public Delegate[] GetInvocationList() => (Delegate[])targets.Clone();

    protected override Delegate CombineImpl(Delegate? d)
    {
        if (d == null) return this;
        MulticastDelegate other = (MulticastDelegate)d;

        MulticastDelegate newDelegate = (MulticastDelegate)MemberwiseClone();
        Delegate[] newTargets = new Delegate[this.targets.Length + other.targets.Length];
        System.Array.Copy(this.targets, 0, newTargets, 0, this.targets.Length);
        System.Array.Copy(other.targets, 0, newTargets, this.targets.Length, other.targets.Length);
        newDelegate.targets = newTargets;
        return newDelegate;
    }

    protected override Delegate? RemoveImpl(Delegate? d)
    {
        if (d == null) return this;
        MulticastDelegate other = (MulticastDelegate)d;

        int thisLen = targets.Length;
        int otherLen = other.targets.Length;

        if (otherLen == 0) return this;

        // ECMA-335: find last occurrence of other.targets sub-sequence in this.targets
        int foundIndex = -1;
        for (int i = thisLen - otherLen; i >= 0; i--)
        {
            bool match = true;
            for (int j = 0; j < otherLen; j++)
            {
                if (!InvocationEntryEquals(targets[i + j], other.targets[j]))
                {
                    match = false;
                    break;
                }
            }
            if (match)
            {
                foundIndex = i;
                break;
            }
        }

        if (foundIndex == -1) return this;

        int newLen = thisLen - otherLen;
        if (newLen == 0) return null;

        MulticastDelegate newDelegate = (MulticastDelegate)MemberwiseClone();
        Delegate[] newTargets = new Delegate[newLen];
        if (foundIndex > 0)
        {
            System.Array.Copy(targets, 0, newTargets, 0, foundIndex);
        }
        if (foundIndex < newLen)
        {
            System.Array.Copy(targets, foundIndex + otherLen, newTargets, foundIndex, newLen - foundIndex);
        }
        newDelegate.targets = newTargets;
        return newDelegate;
    }

    private static bool InvocationEntryEquals(Delegate a, Delegate b)
    {
        return a.Target == b.Target && a._method.Equals(b._method);
    }
}