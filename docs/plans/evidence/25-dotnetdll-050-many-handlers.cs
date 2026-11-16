using System;
using System.Reflection.Emit;
class Program {
 static int Main() {
  var m = new DynamicMethod("Many", typeof(int), Type.EmptyTypes); var il = m.GetILGenerator();
  var local = il.DeclareLocal(typeof(int));
  il.BeginExceptionBlock();
  il.Emit(OpCodes.Newobj, typeof(Exception).GetConstructor(Type.EmptyTypes)!);
  il.Emit(OpCodes.Throw);
  for (int i = 0; i < 21; i++) {
   il.BeginCatchBlock(typeof(Exception)); il.Emit(OpCodes.Pop);
   il.Emit(OpCodes.Ldc_I4, 42 + i); il.Emit(OpCodes.Stloc, local);
  }
  il.EndExceptionBlock(); il.Emit(OpCodes.Ldloc, local); il.Emit(OpCodes.Ret);
  int value = ((Func<int>)m.CreateDelegate(typeof(Func<int>)))(); Console.WriteLine(value); return value;
 }
}
