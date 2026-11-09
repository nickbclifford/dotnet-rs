using System;
using System.Threading;

public static class Program
{
    static bool staticTaken;

    static bool ThrowsExactly<TException>(Action action) where TException : Exception
    {
        try
        {
            action();
        }
        catch (Exception exception)
        {
            return exception.GetType() == typeof(TException);
        }

        return false;
    }

    static bool IsNotHeld(object target) =>
        ThrowsExactly<SynchronizationLockException>(() => Monitor.Exit(target));

    public static int Main()
    {
        object gate = new object();

        bool taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.Enter(gate, ref taken)) || !taken || !IsNotHeld(gate))
            return 1;
        taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.TryEnter(gate, ref taken)) || !taken || !IsNotHeld(gate))
            return 2;
        taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.TryEnter(gate, 0, ref taken)) || !taken || !IsNotHeld(gate))
            return 3;
        taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.TryEnter(gate, TimeSpan.Zero, ref taken)) || !taken || !IsNotHeld(gate))
            return 4;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Monitor.TryEnter(gate, -2)) || !IsNotHeld(gate))
            return 5;
        taken = false;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Monitor.TryEnter(gate, -2, ref taken)) || taken || !IsNotHeld(gate))
            return 6;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Monitor.TryEnter(gate, TimeSpan.FromMilliseconds(-2))) || !IsNotHeld(gate))
            return 7;
        taken = false;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Monitor.TryEnter(gate, TimeSpan.FromMilliseconds(-2), ref taken)) || taken || !IsNotHeld(gate))
            return 8;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.Enter(null)))
            return 9;
        taken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.Enter(null, ref taken)) || taken)
            return 10;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null)))
            return 11;
        taken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, ref taken)) || taken)
            return 12;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, 0)))
            return 13;
        taken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, 0, ref taken)) || taken)
            return 14;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, TimeSpan.Zero)))
            return 15;
        taken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, TimeSpan.Zero, ref taken)) || taken)
            return 16;

        // A static byref is excluded from native effects and uses the synchronous fallback.
        staticTaken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.Enter(gate, ref staticTaken)) || !staticTaken || !IsNotHeld(gate))
            return 17;
        staticTaken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.TryEnter(gate, 0, ref staticTaken)) || !staticTaken || !IsNotHeld(gate))
            return 18;
        staticTaken = false;
        if (!ThrowsExactly<ArgumentOutOfRangeException>(() => Monitor.TryEnter(gate, -2, ref staticTaken)) || staticTaken || !IsNotHeld(gate))
            return 19;
        staticTaken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, 0, ref staticTaken)) || staticTaken)
            return 20;
        staticTaken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.Enter(null, ref staticTaken)) || staticTaken)
            return 21;

        // Check CLR precedence when more than one argument is invalid.
        taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.Enter(null, ref taken)) || !taken)
            return 22;
        taken = true;
        if (!ThrowsExactly<ArgumentException>(() => Monitor.TryEnter(null, -2, ref taken)) || !taken)
            return 23;
        taken = false;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, -2, ref taken)) || taken)
            return 24;
        if (!ThrowsExactly<ArgumentNullException>(() => Monitor.TryEnter(null, -2)))
            return 25;

        Monitor.Enter(gate);
        Monitor.Exit(gate);
        if (!Monitor.TryEnter(gate))
            return 26;
        Monitor.Exit(gate);
        return 42;
    }
}
