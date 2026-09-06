import {expect,test} from '@playwright/test'
import {fieldKey,fieldTestId,calculationMatchesSubset,calculationListKey} from '../../src/fields'

test('canonical field identity separates literal properties, independent results and derived columns',()=>{
  const fields = [{kind:'property',name:'total'}, {kind:'property',name:'@derived:degree:total'}, {kind:'property',name:'derived:["degree","total"]'}, {kind:'derived',calculation_id:'degree',column:'total'}, {kind:'derived',calculation_id:'degree-2',column:'total'}, {kind:'derived',calculation_id:'degree',column:'in'}] as const
  expect(new Set(fields.map(fieldKey)).size).toBe(fields.length)
  expect(fieldTestId({kind:'property',name:'id'})).toBe('id')
})

test('a restored frozen input never matches a new generation with the same subset revision',()=>{
  const calculation = {input_stamp:{generation:'old',revision:'7'},input_subset_revision:'1'}
  expect(calculationMatchesSubset(calculation,{stamp:{generation:'old',revision:'8'},subset_revision:'1'})).toBe(true)
  expect(calculationMatchesSubset(calculation,{stamp:{generation:'new',revision:'8'},subset_revision:'1'})).toBe(false)
  expect(calculationMatchesSubset(calculation,{stamp:{generation:'old',revision:'8'},subset_revision:'2'})).toBe(false)
})

test('calculation list cache refreshes its frozen-scope description on generation replacement',()=>{
  const calculation = {id:'degree-1',kind:'degree' as const,input_stamp:{generation:'old',revision:'7'},input_subset_revision:'1',scope:'visible',node_count:3,edge_count:2,fields:[],status:'ready',elapsed_ms:1,semantics:'Directed degree'}
  const previous = {stamp:{generation:'old',revision:'8'},subset_revision:'1',calculations:[calculation]}
  const replaced = {...previous,stamp:{generation:'new',revision:'8'}}
  expect(calculationMatchesSubset(calculation,previous)).toBe(true)
  expect(calculationMatchesSubset(calculation,replaced)).toBe(false)
  expect(calculationListKey(replaced)).not.toBe(calculationListKey(previous))
})
