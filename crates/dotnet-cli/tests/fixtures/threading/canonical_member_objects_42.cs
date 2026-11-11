using System;
using System.Reflection;
using System.Threading;

class WorkerOwned
{
    public int Field;
    public int Property { get; set; }
    public void Method() { }
}
class MainOwned
{
    public int Field;
    public int Property { get; set; }
    public void Method() { }
}
class RaceOwned
{
    public int Field;
    public int Property { get; set; }
    public void Method() { }
}

static class Program
{
    static MethodInfo? workerMethod, mainMethod, raceMethodA, raceMethodB;
    static ConstructorInfo? workerCtor, mainCtor, raceCtorA, raceCtorB;
    static FieldInfo? workerField, mainField, raceFieldA, raceFieldB;
    static PropertyInfo? workerProperty, mainProperty, racePropertyA, racePropertyB;
    static Assembly? workerAssembly, mainAssembly;
    static volatile bool start;
    static bool workerOk;

    static void Race(bool first)
    {
        while (!start) Thread.Sleep(0);
        Type t = typeof(RaceOwned);
        var method = t.GetMethod("Method");
        var ctor = t.GetConstructor(Type.EmptyTypes);
        var field = t.GetField("Field");
        var property = t.GetProperty("Property");
        if (first) { raceMethodA = method; raceCtorA = ctor; raceFieldA = field; racePropertyA = property; }
        else { raceMethodB = method; raceCtorB = ctor; raceFieldB = field; racePropertyB = property; }
    }

    static int Main()
    {
        Type main = typeof(MainOwned);
        mainMethod = main.GetMethod("Method");
        mainCtor = main.GetConstructor(Type.EmptyTypes);
        mainField = main.GetField("Field");
        mainProperty = main.GetProperty("Property");
        mainAssembly = typeof(string).Assembly;
        var creator = new Thread(() =>
        {
            // Use the foreign assembly before looking it up in this thread.
            bool nameOk = mainAssembly!.GetName().Name == "System.Private.CoreLib";
            Type t = typeof(MainOwned);
            workerOk = nameOk && ReferenceEquals(mainMethod, t.GetMethod("Method"))
                && ReferenceEquals(mainCtor, t.GetConstructor(Type.EmptyTypes))
                && ReferenceEquals(mainField, t.GetField("Field"))
                && ReferenceEquals(mainProperty, t.GetProperty("Property"))
                && ReferenceEquals(mainAssembly, typeof(string).Assembly);
            t = typeof(WorkerOwned);
            workerMethod = t.GetMethod("Method");
            workerCtor = t.GetConstructor(Type.EmptyTypes);
            workerField = t.GetField("Field");
            workerProperty = t.GetProperty("Property");
            workerAssembly = t.Assembly;
        });
        creator.Start(); creator.Join();
        GC.Collect(); GC.WaitForPendingFinalizers();
        // Resolve this foreign object before calling Type.Assembly in the main thread.
        bool ok = workerOk && workerAssembly!.GetName().Name != null
            && workerMethod != null && workerCtor != null && workerField != null && workerProperty != null;
        Type worker = typeof(WorkerOwned);
        ok = ok && ReferenceEquals(workerMethod, worker.GetMethod("Method"))
            && ReferenceEquals(workerCtor, worker.GetConstructor(Type.EmptyTypes))
            && ReferenceEquals(workerField, worker.GetField("Field"))
            && ReferenceEquals(workerProperty, worker.GetProperty("Property"))
            && ReferenceEquals(workerAssembly, worker.Assembly)
            && workerMethod!.Name == "Method" && workerField!.Name == "Field"
            && workerProperty!.Name == "Property" && workerProperty.PropertyType == typeof(int)
            && workerProperty.GetGetMethod()!.Name == "get_Property";
        var a = new Thread(() => Race(true));
        var b = new Thread(() => Race(false));
        a.Start(); b.Start(); start = true; a.Join(); b.Join();
        GC.Collect();
        ok = ok && raceMethodA != null && raceCtorA != null && raceFieldA != null && racePropertyA != null
            && ReferenceEquals(raceMethodA, raceMethodB) && ReferenceEquals(raceCtorA, raceCtorB)
            && ReferenceEquals(raceFieldA, raceFieldB) && ReferenceEquals(racePropertyA, racePropertyB)
            && racePropertyA!.Name == "Property" && racePropertyA.PropertyType == typeof(int);
        Console.WriteLine(ok);
        return ok ? 42 : 1;
    }
}
