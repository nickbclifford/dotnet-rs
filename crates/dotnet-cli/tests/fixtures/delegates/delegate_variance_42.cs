using System;

public class Base { }
public class Derived : Base { }

public delegate Base CovariantDelegate();
public delegate void ContravariantDelegate(Derived d);

public class Program {
    public static Derived GetDerived() => new Derived();
    public static void HandleBase(Base b) { }

    public static int Main() {
        // Covariance: Method returns Derived, delegate expects Base
        CovariantDelegate co = new CovariantDelegate(GetDerived);
        Base b = co();
        if (b == null) return 1;

        // Contravariance: Method takes Base, delegate takes Derived
        ContravariantDelegate contra = new ContravariantDelegate(HandleBase);
        contra(new Derived());

        return 42;
    }
}
