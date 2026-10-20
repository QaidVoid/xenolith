//! The instruction numbers a SPIR-V module is written from.
//!
//! A module is a run of words and each instruction starts with its number
//! packed beside how many words it takes. Written as bare integers, a reader
//! cannot tell what an instruction is without the specification open, and a
//! mistake in the number is invisible: the module is the same length either
//! way, and only a validator would say so.
//!
//! Every name here is the one the specification gives that number, and every
//! number here is one this crate writes. A name for an instruction nothing
//! emits would be a claim nobody checks.

/// The extended instruction set the shader arithmetic comes from.
pub const GLSL_STANDARD: &str = "GLSL.std.450";

/// `OpExtInstImport`: names an extended instruction set and gives it an id.
pub const EXT_INST_IMPORT: u16 = 11;

/// `OpExtInst`: runs one instruction from a set named by an earlier import.
pub const EXT_INST: u16 = 12;

/// `OpTypeVoid`.
pub const TYPE_VOID: u16 = 19;

/// `OpTypeBool`.
pub const TYPE_BOOL: u16 = 20;

/// `OpTypeInt`: a whole number of a stated width, signed or not.
pub const TYPE_INT: u16 = 21;

/// `OpTypeFloat`: a number of a stated width.
pub const TYPE_FLOAT: u16 = 22;

/// `OpTypeVector`: several of one component type.
pub const TYPE_VECTOR: u16 = 23;

/// `OpTypeImage`: something that can be read from, such as a texture.
pub const TYPE_IMAGE: u16 = 25;

/// `OpTypeSampledImage`: an image together with the sampler that reads it.
pub const TYPE_SAMPLED_IMAGE: u16 = 27;

/// `OpTypeArray`: a run of one component type of a stated length.
pub const TYPE_ARRAY: u16 = 28;

/// `OpTypeRuntimeArray`: a run whose length is not known when the module is written.
pub const TYPE_RUNTIME_ARRAY: u16 = 29;

/// `OpTypeStruct`: several values under one type.
pub const TYPE_STRUCT: u16 = 30;

/// `OpTypePointer`: what the shader reaches something through, and which storage it lives in.
pub const TYPE_POINTER: u16 = 32;

/// `OpTypeFunction`: the signature of a function.
pub const TYPE_FUNCTION: u16 = 33;

/// `OpConstantFalse`.
pub const CONSTANT_FALSE: u16 = 42;

/// `OpConstant`: one value of a type, written into the module.
pub const CONSTANT: u16 = 43;

/// `OpConstantComposite`: one value built out of several written into the module.
pub const CONSTANT_COMPOSITE: u16 = 44;

/// `OpConstantNull`: the zero of a type, whatever zero means for it.
pub const CONSTANT_NULL: u16 = 46;

/// `OpSpecConstant`: a value a driver may replace before the module is used.
pub const SPEC_CONSTANT: u16 = 50;

/// `OpFunction`: starts a function, naming its result type and signature.
pub const FUNCTION: u16 = 54;

/// `OpFunctionEnd`.
pub const FUNCTION_END: u16 = 56;

/// `OpVariable`: something a function or a module keeps and reaches by pointer.
pub const VARIABLE: u16 = 59;

/// `OpLoad`: reads through a pointer.
pub const LOAD: u16 = 61;

/// `OpStore`: writes through a pointer.
pub const STORE: u16 = 62;

/// `OpAccessChain`: works out a pointer into something, given indices.
pub const ACCESS_CHAIN: u16 = 65;

/// `OpVectorShuffle`: takes the components of two vectors in a stated order.
pub const VECTOR_SHUFFLE: u16 = 79;

/// `OpCompositeConstruct`: builds one value out of several.
pub const COMPOSITE_CONSTRUCT: u16 = 80;

/// `OpCompositeExtract`: takes one part out of a composite value.
pub const COMPOSITE_EXTRACT: u16 = 81;

/// `OpImageSampleImplicitLod`: reads a picture, letting the driver choose the level.
pub const IMAGE_SAMPLE_IMPLICIT_LOD: u16 = 87;

/// `OpImageSampleExplicitLod`: reads a picture at a level the shader states.
pub const IMAGE_SAMPLE_EXPLICIT_LOD: u16 = 88;

/// `OpConvertFToU`: a number with a point in it as an unsigned whole number.
pub const CONVERT_FLOAT_TO_UNSIGNED: u16 = 109;

/// `OpConvertSToF`: a signed whole number as a number with a point in it.
pub const CONVERT_SIGNED_TO_FLOAT: u16 = 111;

/// `OpConvertUToF`: an unsigned whole number as a number with a point in it.
pub const CONVERT_UNSIGNED_TO_FLOAT: u16 = 112;

/// `OpBitcast`: the same bits read as another type.
pub const BITCAST: u16 = 124;

/// `OpFNegate`.
pub const FLOAT_NEGATE: u16 = 127;

/// `OpIAdd`.
pub const INTEGER_ADD: u16 = 128;

/// `OpFAdd`.
pub const FLOAT_ADD: u16 = 129;

/// `OpISub`.
pub const INTEGER_SUBTRACT: u16 = 130;

/// `OpFSub`.
pub const FLOAT_SUBTRACT: u16 = 131;

/// `OpIMul`.
pub const INTEGER_MULTIPLY: u16 = 132;

/// `OpFMul`.
pub const FLOAT_MULTIPLY: u16 = 133;

/// `OpFDiv`.
pub const FLOAT_DIVIDE: u16 = 136;

/// `OpDot`: the sum of the products of two vectors.
pub const DOT: u16 = 148;

/// `OpAny`: whether any component of a vector holds.
pub const ANY: u16 = 154;

/// `OpLogicalOr`.
pub const LOGICAL_OR: u16 = 166;

/// `OpLogicalAnd`.
pub const LOGICAL_AND: u16 = 167;

/// `OpLogicalNot`.
pub const LOGICAL_NOT: u16 = 168;

/// `OpSelect`: one of two values, chosen by a condition.
pub const SELECT: u16 = 169;

/// `OpIEqual`.
pub const INTEGER_EQUAL: u16 = 170;

/// `OpINotEqual`.
pub const INTEGER_NOT_EQUAL: u16 = 171;

/// `OpFOrdEqual`: equal, and both ordered, which is false where either is not a number.
pub const FLOAT_ORDERED_EQUAL: u16 = 180;

/// `OpFOrdLessThanEqual`.
pub const FLOAT_ORDERED_LESS_THAN_EQUAL: u16 = 188;

/// `OpFOrdGreaterThanEqual`.
pub const FLOAT_ORDERED_GREATER_THAN_EQUAL: u16 = 190;

/// `OpShiftRightLogical`.
pub const SHIFT_RIGHT_LOGICAL: u16 = 194;

/// `OpBitwiseAnd`.
pub const BITWISE_AND: u16 = 199;

/// `OpLoopMerge`: marks the head of a loop, with where it continues and where it leaves.
pub const LOOP_MERGE: u16 = 246;

/// `OpSelectionMerge`: marks where a selection rejoins.
pub const SELECTION_MERGE: u16 = 247;

/// `OpLabel`: where execution may arrive.
pub const LABEL: u16 = 248;

/// `OpBranch`: go to a label.
pub const BRANCH: u16 = 249;

/// `OpBranchConditional`: go to one of two labels on a condition.
pub const BRANCH_CONDITIONAL: u16 = 250;

/// `OpSwitch`: go to the label the selector names.
pub const SWITCH: u16 = 251;

/// `OpKill`: stop this invocation without writing anything.
pub const KILL: u16 = 252;

/// `OpReturn`.
pub const RETURN: u16 = 253;

// The operations of the extended set, which are named in the operand of an
// `OpExtInst` rather than being instructions of their own.

/// `Trunc`: towards zero.
pub const GLSL_TRUNC: u32 = 3;

/// `FAbs`.
pub const GLSL_ABSOLUTE: u32 = 4;

/// `Floor`.
pub const GLSL_FLOOR: u32 = 8;

/// `Fract`: the part after the point.
pub const GLSL_FRACT: u32 = 10;

/// `FMin`.
pub const GLSL_MINIMUM: u32 = 37;

/// `FMax`.
pub const GLSL_MAXIMUM: u32 = 40;

/// `FClamp`.
pub const GLSL_CLAMP: u32 = 43;

/// `Fma`: multiplies two and adds a third, rounding once.
pub const GLSL_FUSED_MULTIPLY_ADD: u32 = 50;

/// `UnpackHalf2x16`: one word as two half sized floats.
pub const GLSL_UNPACK_HALF2X16: u32 = 62;

/// `OpFUnordNotEqual`: not equal, or either side is not a number.
pub const FLOAT_UNORDERED_NOT_EQUAL: u16 = 183;

/// `OpFOrdGreaterThan`.
pub const FLOAT_ORDERED_GREATER_THAN: u16 = 186;

/// `OpUnreachable`: a place execution must never arrive at.
pub const UNREACHABLE: u16 = 255;

/// `OpBitFieldSExtract`: a run of bits taken out of a word and sign extended.
pub const BIT_FIELD_SIGNED_EXTRACT: u16 = 202;

/// `OpBitFieldUExtract`: a run of bits taken out of a word, zero extended.
pub const BIT_FIELD_UNSIGNED_EXTRACT: u16 = 203;

/// `OpFOrdNotEqual`: not equal, and both ordered.
pub const FLOAT_ORDERED_NOT_EQUAL: u16 = 182;

/// `OpFOrdLessThan`.
pub const FLOAT_ORDERED_LESS_THAN: u16 = 184;
