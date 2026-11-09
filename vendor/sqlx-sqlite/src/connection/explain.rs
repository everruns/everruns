// Bad casts in this module SHOULD NOT result in a SQL injection
// https://github.com/launchbadge/sqlx/issues/3440
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
use crate::connection::intmap::IntMap;
use crate::connection::{execute, ConnectionState};
use crate::error::Error;
use crate::from_row::FromRow;
use crate::logger::{BranchParent, BranchResult, DebugDiff};
use crate::type_info::DataType;
use crate::SqliteTypeInfo;
use sqlx_core::sql_str::AssertSqlSafe;
use sqlx_core::{hash_map, HashMap};
use std::fmt::Debug;
use std::str::from_utf8;

// affinity
const SQLITE_AFF_NONE: u8 = 0x40; /* '@' */
const SQLITE_AFF_BLOB: u8 = 0x41; /* 'A' */
const SQLITE_AFF_TEXT: u8 = 0x42; /* 'B' */
const SQLITE_AFF_NUMERIC: u8 = 0x43; /* 'C' */
const SQLITE_AFF_INTEGER: u8 = 0x44; /* 'D' */
const SQLITE_AFF_REAL: u8 = 0x45; /* 'E' */

// opcodes
const OP_INIT: &str = "Init";
const OP_GOTO: &str = "Goto";
const OP_DECR_JUMP_ZERO: &str = "DecrJumpZero";
const OP_DELETE: &str = "Delete";
const OP_ELSE_EQ: &str = "ElseEq";
const OP_EQ: &str = "Eq";
const OP_END_COROUTINE: &str = "EndCoroutine";
const OP_FILTER: &str = "Filter";
const OP_FK_IF_ZERO: &str = "FkIfZero";
const OP_FOUND: &str = "Found";
const OP_GE: &str = "Ge";
const OP_GO_SUB: &str = "Gosub";
const OP_GT: &str = "Gt";
const OP_IDX_GE: &str = "IdxGE";
const OP_IDX_GT: &str = "IdxGT";
const OP_IDX_LE: &str = "IdxLE";
const OP_IDX_LT: &str = "IdxLT";
const OP_IDX_ROWID: &str = "IdxRowid";
const OP_IF: &str = "If";
const OP_IF_NO_HOPE: &str = "IfNoHope";
const OP_IF_NOT: &str = "IfNot";
const OP_IF_NOT_OPEN: &str = "IfNotOpen";
const OP_IF_NOT_ZERO: &str = "IfNotZero";
const OP_IF_NULL_ROW: &str = "IfNullRow";
const OP_IF_POS: &str = "IfPos";
const OP_IF_SMALLER: &str = "IfSmaller";
const OP_INCR_VACUUM: &str = "IncrVacuum";
const OP_INIT_COROUTINE: &str = "InitCoroutine";
const OP_IS_NULL: &str = "IsNull";
const OP_IS_NULL_OR_TYPE: &str = "IsNullOrType";
const OP_LAST: &str = "Last";
const OP_LE: &str = "Le";
const OP_LT: &str = "Lt";
const OP_MUST_BE_INT: &str = "MustBeInt";
const OP_NE: &str = "Ne";
const OP_NEXT: &str = "Next";
const OP_NO_CONFLICT: &str = "NoConflict";
const OP_NOT_EXISTS: &str = "NotExists";
const OP_NOT_NULL: &str = "NotNull";
const OP_ONCE: &str = "Once";
const OP_PREV: &str = "Prev";
const OP_PROGRAM: &str = "Program";
const OP_RETURN: &str = "Return";
const OP_REWIND: &str = "Rewind";
const OP_ROW_DATA: &str = "RowData";
const OP_ROW_SET_READ: &str = "RowSetRead";
const OP_ROW_SET_TEST: &str = "RowSetTest";
const OP_SEEK_GE: &str = "SeekGE";
const OP_SEEK_GT: &str = "SeekGT";
const OP_SEEK_LE: &str = "SeekLE";
const OP_SEEK_LT: &str = "SeekLT";
const OP_SEEK_ROW_ID: &str = "SeekRowid";
const OP_SEEK_SCAN: &str = "SeekScan";
const OP_SEQUENCE: &str = "Sequence";
const OP_SEQUENCE_TEST: &str = "SequenceTest";
const OP_SORT: &str = "Sort";
const OP_SORTER_DATA: &str = "SorterData";
const OP_SORTER_INSERT: &str = "SorterInsert";
const OP_SORTER_NEXT: &str = "SorterNext";
const OP_SORTER_OPEN: &str = "SorterOpen";
const OP_SORTER_SORT: &str = "SorterSort";
const OP_V_FILTER: &str = "VFilter";
const OP_V_NEXT: &str = "VNext";
const OP_YIELD: &str = "Yield";
const OP_JUMP: &str = "Jump";
const OP_COLUMN: &str = "Column";
const OP_MAKE_RECORD: &str = "MakeRecord";
const OP_INSERT: &str = "Insert";
const OP_IDX_INSERT: &str = "IdxInsert";
const OP_OPEN_DUP: &str = "OpenDup";
const OP_OPEN_PSEUDO: &str = "OpenPseudo";
const OP_OPEN_READ: &str = "OpenRead";
const OP_OPEN_WRITE: &str = "OpenWrite";
const OP_OPEN_EPHEMERAL: &str = "OpenEphemeral";
const OP_OPEN_AUTOINDEX: &str = "OpenAutoindex";
const OP_AGG_FINAL: &str = "AggFinal";
const OP_AGG_VALUE: &str = "AggValue";
const OP_AGG_STEP: &str = "AggStep";
const OP_FUNCTION: &str = "Function";
const OP_MOVE: &str = "Move";
const OP_COPY: &str = "Copy";
const OP_SCOPY: &str = "SCopy";
const OP_NULL: &str = "Null";
const OP_NULL_ROW: &str = "NullRow";
const OP_INT_COPY: &str = "IntCopy";
const OP_CAST: &str = "Cast";
const OP_STRING8: &str = "String8";
const OP_INT64: &str = "Int64";
const OP_INTEGER: &str = "Integer";
const OP_REAL: &str = "Real";
const OP_NOT: &str = "Not";
const OP_BLOB: &str = "Blob";
const OP_VARIABLE: &str = "Variable";
const OP_COUNT: &str = "Count";
const OP_ROWID: &str = "Rowid";
const OP_NEWROWID: &str = "NewRowid";
const OP_OR: &str = "Or";
const OP_AND: &str = "And";
const OP_BIT_AND: &str = "BitAnd";
const OP_BIT_OR: &str = "BitOr";
const OP_SHIFT_LEFT: &str = "ShiftLeft";
const OP_SHIFT_RIGHT: &str = "ShiftRight";
const OP_ADD: &str = "Add";
const OP_SUBTRACT: &str = "Subtract";
const OP_MULTIPLY: &str = "Multiply";
const OP_DIVIDE: &str = "Divide";
const OP_REMAINDER: &str = "Remainder";
const OP_CONCAT: &str = "Concat";
const OP_OFFSET_LIMIT: &str = "OffsetLimit";
const OP_RESULT_ROW: &str = "ResultRow";
const OP_HALT: &str = "Halt";
const OP_HALT_IF_NULL: &str = "HaltIfNull";

const MAX_LOOP_COUNT: u8 = 2;
const MAX_TOTAL_INSTRUCTION_COUNT: u32 = 100_000;

#[derive(Clone, Eq, PartialEq, Hash)]
enum ColumnType {
    Single {
        datatype: DataType,
        nullable: Option<bool>,
    },
    Record(IntMap<ColumnType>),
}

impl Default for ColumnType {
    fn default() -> Self {
        Self::Single {
            datatype: DataType::Null,
            nullable: None,
        }
    }
}

impl ColumnType {
    fn null() -> Self {
        Self::Single {
            datatype: DataType::Null,
            nullable: Some(true),
        }
    }
    fn map_to_datatype(&self) -> DataType {
        match self {
            Self::Single { datatype, .. } => *datatype,
            Self::Record(_) => DataType::Null, //If we're trying to coerce to a regular Datatype, we can assume a Record is invalid for the context
        }
    }
    fn map_to_nullable(&self) -> Option<bool> {
        match self {
            Self::Single { nullable, .. } => *nullable,
            Self::Record(_) => None, //If we're trying to coerce to a regular Datatype, we can assume a Record is invalid for the context
        }
    }
}

impl core::fmt::Debug for ColumnType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Single { datatype, nullable } => {
                let nullable_str = match nullable {
                    Some(true) => "NULL",
                    Some(false) => "NOT NULL",
                    None => "NULL?",
                };
                write!(f, "{:?} {}", datatype, nullable_str)
            }
            Self::Record(columns) => {
                f.write_str("Record(")?;
                let mut column_iter = columns.iter();
                if let Some(item) = column_iter.next() {
                    write!(f, "{:?}", item)?;
                    for item in column_iter {
                        write!(f, ", {:?}", item)?;
                    }
                }
                f.write_str(")")
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum RegDataType {
    Single(ColumnType),
    Int(i64),
}

impl RegDataType {
    fn map_to_datatype(&self) -> DataType {
        match self {
            RegDataType::Single(d) => d.map_to_datatype(),
            RegDataType::Int(_) => DataType::Integer,
        }
    }
    fn map_to_nullable(&self) -> Option<bool> {
        match self {
            RegDataType::Single(d) => d.map_to_nullable(),
            RegDataType::Int(_) => Some(false),
        }
    }
    fn map_to_columntype(&self) -> ColumnType {
        match self {
            RegDataType::Single(d) => d.clone(),
            RegDataType::Int(_) => ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(false),
            },
        }
    }
}

impl Default for RegDataType {
    fn default() -> Self {
        Self::Single(ColumnType::default())
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct TableDataType {
    cols: IntMap<ColumnType>,
    is_empty: Option<bool>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum CursorDataType {
    Normal(i64),
    Pseudo(i64),
}

impl CursorDataType {
    fn columns(
        &self,
        tables: &IntMap<TableDataType>,
        registers: &IntMap<RegDataType>,
    ) -> IntMap<ColumnType> {
        match self {
            Self::Normal(i) => match tables.get(i) {
                Some(tab) => tab.cols.clone(),
                None => IntMap::new(),
            },
            Self::Pseudo(i) => match registers.get(i) {
                Some(RegDataType::Single(ColumnType::Record(r))) => r.clone(),
                _ => IntMap::new(),
            },
        }
    }

    fn columns_ref<'s, 'r, 'o>(
        &'s self,
        tables: &'r IntMap<TableDataType>,
        registers: &'r IntMap<RegDataType>,
    ) -> Option<&'o IntMap<ColumnType>>
    where
        's: 'o,
        'r: 'o,
    {
        match self {
            Self::Normal(i) => match tables.get(i) {
                Some(tab) => Some(&tab.cols),
                None => None,
            },
            Self::Pseudo(i) => match registers.get(i) {
                Some(RegDataType::Single(ColumnType::Record(r))) => Some(r),
                _ => None,
            },
        }
    }

    fn columns_mut<'s, 'r, 'o>(
        &'s self,
        tables: &'r mut IntMap<TableDataType>,
        registers: &'r mut IntMap<RegDataType>,
    ) -> Option<&'o mut IntMap<ColumnType>>
    where
        's: 'o,
        'r: 'o,
    {
        match self {
            Self::Normal(i) => match tables.get_mut(i) {
                Some(tab) => Some(&mut tab.cols),
                None => None,
            },
            Self::Pseudo(i) => match registers.get_mut(i) {
                Some(RegDataType::Single(ColumnType::Record(r))) => Some(r),
                _ => None,
            },
        }
    }

    fn table_mut<'s, 'r, 'o>(
        &'s self,
        tables: &'r mut IntMap<TableDataType>,
    ) -> Option<&'o mut TableDataType>
    where
        's: 'o,
        'r: 'o,
    {
        match self {
            Self::Normal(i) => match tables.get_mut(i) {
                Some(tab) => Some(tab),
                None => None,
            },
            _ => None,
        }
    }

    fn is_empty(&self, tables: &IntMap<TableDataType>) -> Option<bool> {
        match self {
            Self::Normal(i) => match tables.get(i) {
                Some(tab) => tab.is_empty,
                None => Some(true),
            },
            Self::Pseudo(_) => Some(false), //pseudo cursors have exactly one row
        }
    }
}

#[allow(clippy::wildcard_in_or_patterns)]
fn affinity_to_type(affinity: u8) -> DataType {
    match affinity {
        SQLITE_AFF_BLOB => DataType::Blob,
        SQLITE_AFF_INTEGER => DataType::Integer,
        SQLITE_AFF_NUMERIC => DataType::Numeric,
        SQLITE_AFF_REAL => DataType::Float,
        SQLITE_AFF_TEXT => DataType::Text,

        SQLITE_AFF_NONE | _ => DataType::Null,
    }
}

#[allow(clippy::wildcard_in_or_patterns)]
fn opcode_to_type(op: &str) -> DataType {
    match op {
        OP_REAL => DataType::Float,
        OP_BLOB => DataType::Blob,
        OP_AND | OP_OR => DataType::Bool,
        OP_NEWROWID | OP_IDX_ROWID | OP_ROWID | OP_COUNT | OP_INT64 | OP_INTEGER => {
            DataType::Integer
        }
        OP_STRING8 => DataType::Text,
        OP_COLUMN | _ => DataType::Null,
    }
}

fn root_block_columns(
    conn: &mut ConnectionState,
) -> Result<HashMap<(i64, i64), IntMap<ColumnType>>, Error> {
    let table_block_columns: Vec<(i64, i64, i64, String, bool)> = execute::iter(
        conn,
        "SELECT s.dbnum, s.rootpage, col.cid as colnum, col.type, col.\"notnull\"
         FROM (
             select 1 dbnum, tss.* from temp.sqlite_schema tss
             UNION ALL select 0 dbnum, mss.* from main.sqlite_schema mss
             ) s
         JOIN pragma_table_info(s.name) AS col
         WHERE s.type = 'table'
         UNION ALL
         SELECT s.dbnum, s.rootpage, idx.seqno as colnum, col.type, col.\"notnull\"
         FROM (
             select 1 dbnum, tss.* from temp.sqlite_schema tss
             UNION ALL select 0 dbnum, mss.* from main.sqlite_schema mss
             ) s
         JOIN pragma_index_info(s.name) AS idx
         LEFT JOIN pragma_table_info(s.tbl_name) as col
           ON col.cid = idx.cid
           WHERE s.type = 'index'",
        None,
        false,
    )?
    .filter_map(|res| res.map(|either| either.right()).transpose())
    .map(|row| FromRow::from_row(&row?))
    .collect::<Result<Vec<_>, Error>>()?;

    let mut row_info: HashMap<(i64, i64), IntMap<ColumnType>> = HashMap::new();
    for (dbnum, block, colnum, datatype, notnull) in table_block_columns {
        let row_info = row_info.entry((dbnum, block)).or_default();
        row_info.insert(
            colnum,
            ColumnType::Single {
                datatype: datatype.parse().unwrap_or(DataType::Null),
                nullable: Some(!notnull),
            },
        );
    }

    Ok(row_info)
}

struct Sequence(i64);

impl Sequence {
    pub fn new() -> Self {
        Self(0)
    }
    pub fn next(&mut self) -> i64 {
        let curr = self.0;
        self.0 += 1;
        curr
    }
}

#[derive(Debug)]
struct QueryState {
    // The number of times each instruction has been visited
    pub visited: Vec<u8>,
    // A unique identifier of the query branch
    pub branch_id: i64,
    // How many instructions have been executed on this branch (NOT the same as program_i, which is the currently executing instruction of the program)
    pub instruction_counter: i64,
    // Parent branch this branch was forked from (if any)
    pub branch_parent: Option<BranchParent>,
    // State of the virtual machine
    pub mem: MemoryState,
    // Results published by the execution
    pub result: Option<Vec<(Option<SqliteTypeInfo>, Option<bool>)>>,
}

impl From<&QueryState> for MemoryState {
    fn from(val: &QueryState) -> Self {
        val.mem.clone()
    }
}

impl From<QueryState> for MemoryState {
    fn from(val: QueryState) -> Self {
        val.mem
    }
}

impl From<&QueryState> for BranchParent {
    fn from(val: &QueryState) -> Self {
        Self {
            id: val.branch_id,
            idx: val.instruction_counter,
        }
    }
}

impl QueryState {
    fn get_reference(&self) -> BranchParent {
        BranchParent {
            id: self.branch_id,
            idx: self.instruction_counter,
        }
    }
    fn new_branch(&self, branch_seq: &mut Sequence) -> Self {
        Self {
            visited: self.visited.clone(),
            branch_id: branch_seq.next(),
            instruction_counter: 0,
            branch_parent: Some(BranchParent {
                id: self.branch_id,
                idx: self.instruction_counter - 1, //instruction counter is incremented at the start of processing an instruction, so need to subtract 1 to get the 'current' instruction
            }),
            mem: self.mem.clone(),
            result: self.result.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MemoryState {
    // Next instruction to execute
    pub program_i: usize,
    // Registers
    pub r: IntMap<RegDataType>,
    // Rows that pointers point to
    pub p: IntMap<CursorDataType>,
    // Table definitions pointed to by pointers
    pub t: IntMap<TableDataType>,
}

impl DebugDiff for MemoryState {
    fn diff(&self, prev: &Self) -> String {
        let r_diff = self.r.diff(&prev.r);
        let p_diff = self.p.diff(&prev.p);
        let t_diff = self.t.diff(&prev.t);

        let mut differences = String::new();
        for (i, v) in r_diff {
            if !differences.is_empty() {
                differences.push('\n');
            }
            differences.push_str(&format!("r[{}]={:?}", i, v))
        }
        for (i, v) in p_diff {
            if !differences.is_empty() {
                differences.push('\n');
            }
            differences.push_str(&format!("p[{}]={:?}", i, v))
        }
        for (i, v) in t_diff {
            if !differences.is_empty() {
                differences.push('\n');
            }
            differences.push_str(&format!("t[{}]={:?}", i, v))
        }
        differences
    }
}

struct BranchList {
    states: Vec<QueryState>,
    visited_branch_state: HashMap<MemoryState, BranchParent>,
}

impl BranchList {
    pub fn new(state: QueryState) -> Self {
        Self {
            states: vec![state],
            visited_branch_state: HashMap::new(),
        }
    }
    pub fn push<R: Debug, P: Debug>(
        &mut self,
        mut state: QueryState,
        logger: &mut crate::logger::QueryPlanLogger<'_, R, MemoryState, P>,
    ) {
        logger.add_branch(&state, &state.branch_parent.unwrap());
        match self.visited_branch_state.entry(state.mem) {
            hash_map::Entry::Vacant(entry) => {
                //this state is not identical to another state, so it will need to be processed
                state.mem = entry.key().clone(); //replace state.mem since .entry() moved it
                entry.insert(state.get_reference());
                self.states.push(state);
            }
            hash_map::Entry::Occupied(entry) => {
                //already saw a state identical to this one, so no point in processing it
                state.mem = entry.key().clone(); //replace state.mem since .entry() moved it
                logger.add_result(state, BranchResult::Dedup(*entry.get()));
            }
        }
    }
    pub fn pop(&mut self) -> Option<QueryState> {
        self.states.pop()
    }
}

include!("explain/program.rs");

#[test]
fn test_root_block_columns_has_types() {
    use crate::SqliteConnectOptions;
    use std::str::FromStr;
    let conn_options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    let mut conn = super::EstablishParams::from_options(&conn_options)
        .unwrap()
        .establish()
        .unwrap();

    assert!(execute::iter(
        &mut conn,
        r"CREATE TABLE t(a INTEGER PRIMARY KEY, b_null TEXT NULL, b TEXT NOT NULL);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());
    assert!(
        execute::iter(&mut conn, r"CREATE INDEX i1 on t (a,b_null);", None, false)
            .unwrap()
            .next()
            .is_some()
    );
    assert!(execute::iter(
        &mut conn,
        r"CREATE UNIQUE INDEX i2 on t (a,b_null);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());
    assert!(execute::iter(
        &mut conn,
        r"CREATE TABLE t2(a INTEGER NOT NULL, b_null NUMERIC NULL, b NUMERIC NOT NULL);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());
    assert!(execute::iter(
        &mut conn,
        r"CREATE INDEX t2i1 on t2 (a,b_null);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());
    assert!(execute::iter(
        &mut conn,
        r"CREATE UNIQUE INDEX t2i2 on t2 (a,b);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());

    assert!(execute::iter(
        &mut conn,
        r"CREATE TEMPORARY TABLE t3(a TEXT PRIMARY KEY, b REAL NOT NULL, b_null REAL NULL);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());

    let table_block_nums: HashMap<String, (i64,i64)> = execute::iter(
        &mut conn,
        r"select name, 0 db_seq, rootpage from main.sqlite_schema UNION ALL select name, 1 db_seq, rootpage from temp.sqlite_schema",
        None,
        false,
    )
    .unwrap()
    .filter_map(|res| res.map(|either| either.right()).transpose())
    .map(|row| FromRow::from_row(row.as_ref().unwrap()))
    .map(|row| row.map(|(name,seq,block)|(name,(seq,block))))
    .collect::<Result<HashMap<_, _>, Error>>()
    .unwrap();

    let root_block_cols = root_block_columns(&mut conn).unwrap();

    // there should be 7 tables/indexes created explicitly, plus 1 autoindex for t3
    assert_eq!(8, root_block_cols.len());

    //prove that we have some information for each table & index
    for (name, db_seq_block) in dbg!(&table_block_nums) {
        assert!(
            root_block_cols.contains_key(db_seq_block),
            "{:?}",
            (name, db_seq_block)
        );
    }

    //prove that each block has the correct information
    {
        let table_db_block = table_block_nums["t"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(true) //sqlite primary key columns are nullable unless declared not null
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Text,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Text,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&2)
        );
    }

    {
        let table_db_block = table_block_nums["i1"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(true) //sqlite primary key columns are nullable unless declared not null
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Text,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
    }

    {
        let table_db_block = table_block_nums["i2"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(true) //sqlite primary key columns are nullable unless declared not null
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Text,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
    }

    {
        let table_db_block = table_block_nums["t2"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Null,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Null,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&2)
        );
    }

    {
        let table_db_block = table_block_nums["t2i1"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Null,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
    }

    {
        let table_db_block = table_block_nums["t2i2"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Integer,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Null,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
    }

    {
        let table_db_block = table_block_nums["t3"];
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Text,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&0)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Float,
                nullable: Some(false)
            }),
            root_block_cols[&table_db_block].get(&1)
        );
        assert_eq!(
            Some(&ColumnType::Single {
                datatype: DataType::Float,
                nullable: Some(true)
            }),
            root_block_cols[&table_db_block].get(&2)
        );
    }
}

#[test]
fn test_explain() {
    use crate::SqliteConnectOptions;
    use std::str::FromStr;
    let conn_options = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
    let mut conn = super::EstablishParams::from_options(&conn_options)
        .unwrap()
        .establish()
        .unwrap();

    assert!(execute::iter(
        &mut conn,
        r"CREATE TABLE an_alias(a INTEGER PRIMARY KEY);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());

    assert!(execute::iter(
        &mut conn,
        r"CREATE TABLE not_an_alias(a INT PRIMARY KEY);",
        None,
        false
    )
    .unwrap()
    .next()
    .is_some());

    assert!(
        if let Ok((ty, nullable)) = explain(&mut conn, "SELECT * FROM an_alias") {
            ty == [SqliteTypeInfo(DataType::Integer)] && nullable == [Some(false)]
        } else {
            false
        }
    );
    assert!(
        if let Ok((ty, nullable)) = explain(&mut conn, "SELECT * FROM not_an_alias") {
            ty == [SqliteTypeInfo(DataType::Integer)] && nullable == [Some(true)]
        } else {
            false
        }
    );
    assert!(
        if let Ok((ty, nullable)) = explain(&mut conn, "SELECT rowid FROM an_alias") {
            ty == [SqliteTypeInfo(DataType::Integer)] && nullable == [Some(false)]
        } else {
            false
        }
    );
    assert!(
        if let Ok((ty, nullable)) = explain(&mut conn, "SELECT rowid FROM not_an_alias") {
            ty == [SqliteTypeInfo(DataType::Integer)] && nullable == [Some(false)]
        } else {
            false
        }
    );
}
