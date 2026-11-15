using System;
using System.Reflection;
using System.Collections.Generic;

class Target<T> {
    public static int Initialized;
    static Target() { Initialized = 11; }
    public string Result;
    public Target() { GC.Collect(); Result = typeof(T).Name + ":" + Initialized; }
    public string Echo<M>(bool fail) {
        GC.Collect();
        if (fail) throw new InvalidOperationException(typeof(M).Name);
        return typeof(T).Name + "/" + typeof(M).Name;
    }
}
class Parent<T> { public virtual string Describe<M>() => "Parent:" + typeof(T).Name; }
class Child<X> : Parent<List<X>> {
    public override string Describe<M>() { GC.Collect(); return typeof(X).Name + "/" + typeof(M).Name; }
}
class Caller<A> {
    public static string Run<B>() {
        var value = Activator.CreateInstance<Target<int>>();
        var method = typeof(Target<int>).GetMethod("Echo").MakeGenericMethod(typeof(string));
        string result = (string)method.Invoke(value, new object[] { false });
        string noWrap = (string)method.Invoke(value, BindingFlags.DoNotWrapExceptions, null, new object[] { false }, null);
        string noWrapError = "";
        try { method.Invoke(value, BindingFlags.DoNotWrapExceptions, null, new object[] { true }, null); }
        catch (InvalidOperationException ex) { noWrapError = ex.Message; }
        string error = "";
        try { method.Invoke(value, new object[] { true }); }
        catch (TargetInvocationException ex) { error = ex.InnerException.Message; }
        var virtualMethod = typeof(Parent<List<int>>).GetMethod("Describe").MakeGenericMethod(typeof(string));
        string virtualResult = (string)virtualMethod.Invoke(new Child<int>(), null);
        return value.Result + ";" + result + ";" + error + ";" + virtualResult + ";" + noWrap + ";" + noWrapError;
    }
}
public class Program {
    public static int Main() {
        for (int i = 0; i < 2; i++) {
            string value = Caller<long>.Run<double>();
            Console.WriteLine(value);
            if (value != "Int32:11;Int32/String;String;Int32/String;Int32/String;String") return 1;
        }
        return 42;
    }
}
