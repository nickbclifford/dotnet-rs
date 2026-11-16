using System;
using System.Reflection.Emit;
class Program {
 public static int Value = 42;
 static int Main() {
  var m = new DynamicMethod("TailConstrained", typeof(string), Type.EmptyTypes);
  var il = m.GetILGenerator();
  il.Emit(OpCodes.Ldsflda, typeof(Program).GetField(nameof(Value))!);
  il.Emit(OpCodes.Constrained, typeof(int));
  il.Emit(OpCodes.Tailcall);
  il.Emit(OpCodes.Callvirt, typeof(object).GetMethod(nameof(ToString), Type.EmptyTypes)!);
  il.Emit(OpCodes.Ret);
  string result = ((Func<string>)m.CreateDelegate(typeof(Func<string>)))();
  Console.WriteLine(result);
  return int.Parse(result);
 }
}
